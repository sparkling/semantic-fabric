//! MonetDB `SqlBackend` adapter (ADR-0024 M8).
//!
//! Implements the MonetDB MAPI/TCP text protocol directly — no external driver
//! crate required. MonetDB speaks MAPI2 over TCP: a simple challenge-response
//! auth handshake, then SQL queries sent as messages and results streamed back
//! as tab-separated text lines.
//!
//! This implementation is always compiled (no feature gate needed); the MAPI
//! protocol is self-contained and only requires `std::net::TcpStream`.
//!
//! Verification tier: compile + unit (MAPI parsing). Live-parity requires a
//! running MonetDB instance and `SF_MONETDB_URL` set.

use std::collections::VecDeque;
use std::io::{BufReader, Write};
use std::net::TcpStream;

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

/// MonetDB backend — connects via MAPI/TCP and sends SQL as text messages.
///
/// Connection string: `"monet://user:pass@host:50000/database"`
pub struct MonetDbBackend {
    host: String,
    port: u16,
    user: String,
    password: String,
    database: String,
}

impl MonetDbBackend {
    /// Construct from individual connection parameters.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        password: impl Into<String>,
        database: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            password: password.into(),
            database: database.into(),
        }
    }

    /// Parse a `monet://user:pass@host:port/database` URL.
    pub fn from_url(url: &str) -> Result<Self> {
        let url = url.trim_start_matches("monet://");
        // user:pass@host:port/database
        let (auth, rest) = url
            .split_once('@')
            .ok_or_else(|| Error::Marshal("monetdb url: missing '@'".to_owned()))?;
        let (user, password) = auth.split_once(':').unwrap_or((auth, ""));
        let (host_port, database) = rest.split_once('/').unwrap_or((rest, "monetdb"));
        let (host, port_str) = host_port.split_once(':').unwrap_or((host_port, "50000"));
        let port: u16 = port_str.parse().unwrap_or(50000);
        Ok(Self::new(host, port, user, password, database))
    }

    /// From `SF_MONETDB_URL` environment variable.
    pub fn from_env() -> Result<Self> {
        let url = std::env::var("SF_MONETDB_URL")
            .map_err(|_| Error::Marshal("SF_MONETDB_URL not set".to_owned()))?;
        Self::from_url(&url)
    }

    /// Open a fresh TCP connection and perform the MAPI handshake.
    fn connect(&self) -> Result<MapiConn> {
        MapiConn::connect(
            &self.host,
            self.port,
            &self.user,
            &self.password,
            &self.database,
        )
    }
}

/// A MAPI/TCP connection to MonetDB.
struct MapiConn {
    stream: BufReader<TcpStream>,
}

/// MAPI prompt strings (used for future extensions).
#[allow(dead_code)]
const MAPI_READY: &[u8] = b"\n\x01\n";
#[allow(dead_code)]
const MAPI_MORE: &[u8] = b"\n\x02\n";

impl MapiConn {
    /// Connect to MonetDB via MAPI/TCP and authenticate.
    fn connect(host: &str, port: u16, user: &str, password: &str, database: &str) -> Result<Self> {
        let tcp = TcpStream::connect(format!("{host}:{port}"))
            .map_err(|e| Error::Marshal(format!("monetdb tcp connect: {e}")))?;
        let mut reader = BufReader::new(tcp);

        // Read the server challenge.
        let challenge = read_mapi_message(&mut reader)?;
        // Parse challenge: "server:challenge:protocol:hashes:bigendian:database:"
        let parts: Vec<&str> = challenge.split(':').collect();
        if parts.len() < 4 {
            return Err(Error::Marshal(format!(
                "monetdb unexpected challenge: {challenge:?}"
            )));
        }
        let server_challenge = parts[1];
        let hash_algo = parts[3].split(',').next().unwrap_or("MD5");

        // Build response: {hash}:{user}:{hash(password+challenge)}:{database}
        let pw_hash = mapi_hash(hash_algo, password, server_challenge)?;
        let response = format!("{{{}}}:{}:{}:{}:\n", hash_algo, user, pw_hash, database);

        // Send auth response.
        send_mapi_message(reader.get_ref(), response.as_bytes())?;

        // Read the server's reply — should be a READY prompt or an error.
        let reply = read_mapi_message(&mut reader)?;
        if reply.starts_with("!") {
            return Err(Error::Marshal(format!("monetdb auth rejected: {reply}")));
        }

        Ok(MapiConn { stream: reader })
    }

    /// Send a SQL query and return the result as raw text lines.
    fn query(&mut self, sql: &str) -> Result<Vec<String>> {
        let msg = format!("s{sql}\n;\n");
        send_mapi_message(self.stream.get_ref(), msg.as_bytes())?;
        let response = read_mapi_message(&mut self.stream)?;
        Ok(response.lines().map(|l| l.to_owned()).collect())
    }
}

/// Compute the MAPI password hash: `hash_algo(password + challenge)`.
fn mapi_hash(algo: &str, password: &str, challenge: &str) -> Result<String> {
    use std::fmt::Write;
    let input = format!("{password}{challenge}");
    match algo.to_uppercase().as_str() {
        "MD5" => {
            // Pure-Rust MD5 via a simple implementation (no external dep).
            let digest = md5_hash(input.as_bytes());
            let mut out = String::with_capacity(32);
            for b in digest {
                write!(out, "{b:02x}").unwrap();
            }
            Ok(out)
        }
        "SHA1" | "SHA256" | "SHA512" => {
            // Fall back to hexlified first-32-bytes approach for algo we
            // cannot implement inline without a crypto dep. In practice
            // MonetDB defaults to MD5 for the challenge hash.
            Err(Error::Marshal(format!(
                "monetdb auth: {algo} hash not supported; use MD5"
            )))
        }
        other => Err(Error::Marshal(format!(
            "monetdb auth: unknown hash algo {other}"
        ))),
    }
}

/// Pure-Rust MD5 (Rivest 1992). Only used for MAPI auth; not cryptographic
/// quality for other purposes.
fn md5_hash(input: &[u8]) -> [u8; 16] {
    // RFC 1321 constants.
    #[rustfmt::skip]
    const S: [u32; 64] = [
        7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,
        5, 9,14,20,5, 9,14,20,5, 9,14,20,5, 9,14,20,
        4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,
        6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21,
    ];
    #[rustfmt::skip]
    const K: [u32; 64] = [
        0xd76aa478,0xe8c7b756,0x242070db,0xc1bdceee,0xf57c0faf,0x4787c62a,
        0xa8304613,0xfd469501,0x698098d8,0x8b44f7af,0xffff5bb1,0x895cd7be,
        0x6b901122,0xfd987193,0xa679438e,0x49b40821,0xf61e2562,0xc040b340,
        0x265e5a51,0xe9b6c7aa,0xd62f105d,0x02441453,0xd8a1e681,0xe7d3fbc8,
        0x21e1cde6,0xc33707d6,0xf4d50d87,0x455a14ed,0xa9e3e905,0xfcefa3f8,
        0x676f02d9,0x8d2a4c8a,0xfffa3942,0x8771f681,0x6d9d6122,0xfde5380c,
        0xa4beea44,0x4bdecfa9,0xf6bb4b60,0xbebfbc70,0x289b7ec6,0xeaa127fa,
        0xd4ef3085,0x04881d05,0xd9d4d039,0xe6db99e5,0x1fa27cf8,0xc4ac5665,
        0xf4292244,0x432aff97,0xab9423a7,0xfc93a039,0x655b59c3,0x8f0ccc92,
        0xffeff47d,0x85845dd1,0x6fa87e4f,0xfe2ce6e0,0xa3014314,0x4e0811a1,
        0xf7537e82,0xbd3af235,0x2ad7d2bb,0xeb86d391,
    ];

    let orig_len_bits = (input.len() as u64) * 8;
    let mut msg: Vec<u8> = input.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&orig_len_bits.to_le_bytes());

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    for chunk in msg.chunks(64) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            let off = i * 4;
            *w = u32::from_le_bytes([chunk[off], chunk[off + 1], chunk[off + 2], chunk[off + 3]]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0u32..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i as usize])
                    .wrapping_add(m[g as usize])
                    .rotate_left(S[i as usize]),
            );
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

/// Read one complete MAPI message from the stream (may span multiple blocks).
fn read_mapi_message(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut result = String::new();
    loop {
        // MAPI block header: 2 bytes (little-endian length + last-block flag).
        let mut hdr = [0u8; 2];
        use std::io::Read;
        reader
            .read_exact(&mut hdr)
            .map_err(|e| Error::Marshal(format!("monetdb read header: {e}")))?;
        let raw = u16::from_le_bytes(hdr);
        let size = (raw >> 1) as usize;
        let last = (raw & 1) == 1;
        if size > 0 {
            let mut buf = vec![0u8; size];
            reader
                .read_exact(&mut buf)
                .map_err(|e| Error::Marshal(format!("monetdb read block: {e}")))?;
            result.push_str(&String::from_utf8_lossy(&buf));
        }
        if last {
            break;
        }
    }
    Ok(result)
}

/// Send a MAPI message (single block, last=1).
fn send_mapi_message(stream: &TcpStream, data: &[u8]) -> Result<()> {
    let mut tcp = stream
        .try_clone()
        .map_err(|e| Error::Marshal(format!("monetdb tcp clone for write: {e}")))?;
    // MAPI2 block: 2-byte LE header (length*2 | last_flag).
    let len = data.len();
    let raw: u16 = ((len as u16) << 1) | 1;
    tcp.write_all(&raw.to_le_bytes())
        .map_err(|e| Error::Marshal(format!("monetdb write header: {e}")))?;
    tcp.write_all(data)
        .map_err(|e| Error::Marshal(format!("monetdb write body: {e}")))?;
    tcp.flush()
        .map_err(|e| Error::Marshal(format!("monetdb flush: {e}")))?;
    Ok(())
}

/// Parse MAPI result lines into (column_names, row_tuples).
///
/// MAPI result format:
/// - `&` lines: header/table info
/// - `%` lines: column names (`% col1,\tcol2,\t... # name`)
/// - `[` lines: data rows (`[ val1,\tval2,\t... ]`)
/// - `!` lines: error messages
pub fn parse_mapi_result(lines: &[String]) -> Result<(Vec<String>, Vec<RawTuple>)> {
    let mut col_names: Vec<String> = vec![];
    let mut rows: Vec<RawTuple> = vec![];

    for line in lines {
        if line.starts_with('!') {
            return Err(Error::Marshal(format!("monetdb error: {line}")));
        }
        if line.starts_with('%') {
            // Column names line: `% col1,\tcol2,\t... # name`
            if line.contains("# name") {
                let name_part = line.trim_start_matches('%').split('#').next().unwrap_or("");
                col_names = name_part
                    .split(',')
                    .map(|s| s.trim().trim_end_matches('\t').to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        } else if line.starts_with('[') {
            // Data row: `[ val1,\tval2 ]`
            let inner = line.trim_start_matches('[').trim_end_matches(']').trim();
            let ncols = col_names.len().max(1);
            let values: Vec<Option<String>> = inner
                .split(",\t")
                .map(|cell| {
                    let c = cell.trim();
                    if c == "NULL" || c == "nil" {
                        None
                    } else {
                        // Strip surrounding quotes if present.
                        let s = if c.starts_with('"') && c.ends_with('"') {
                            c[1..c.len() - 1].replace("\"\"", "\"")
                        } else {
                            c.to_owned()
                        };
                        Some(s)
                    }
                })
                .collect();
            let mut padded = values;
            padded.resize(ncols, None);
            rows.push(RawTuple {
                codes: vec![None; ncols],
                values: padded,
            });
        }
    }
    Ok((col_names, rows))
}

impl SqlBackend for MonetDbBackend {
    type Stream<'s>
        = MonetDbStream
    where
        Self: 's;

    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
        let mut conn = self.connect()?;
        let lines = conn.query(probe_sql)?;
        let (names, _) = parse_mapi_result(&lines)?;
        Ok(names)
    }

    async fn open_branch(&mut self, sql: &str, lexical_params: &[String]) -> Result<MonetDbStream> {
        // MonetDB MAPI doesn't support prepared-statement bind params over the
        // text protocol; inline them safely (MonetDB values come from trusted
        // SPARQL bind variables, ADR-0010 R1).
        let sql_with_params = inline_params_monetdb(sql, lexical_params);
        let mut conn = self.connect()?;
        let lines = conn.query(&sql_with_params)?;
        let (_, rows) = parse_mapi_result(&lines)?;
        Ok(MonetDbStream { rows: rows.into() })
    }
}

/// A materialised row stream for MonetDB results.
pub struct MonetDbStream {
    rows: VecDeque<RawTuple>,
}

impl BranchStream for MonetDbStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        Ok(self.rows.pop_front())
    }
}

/// Inline `?` positional params into a SQL string using single-quoted literals.
fn inline_params_monetdb(sql: &str, params: &[String]) -> String {
    if params.is_empty() {
        return sql.to_owned();
    }
    let mut result =
        String::with_capacity(sql.len() + params.iter().map(|p| p.len() + 2).sum::<usize>());
    let mut param_iter = params.iter();
    for ch in sql.chars() {
        if ch == '?' {
            if let Some(p) = param_iter.next() {
                result.push('\'');
                result.push_str(&p.replace('\'', "''"));
                result.push('\'');
            } else {
                result.push('?');
            }
        } else {
            result.push(ch);
        }
    }
    result
}

// ─── unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_hello_world() {
        // "hello world" → 5eb63bbbe01eeed093cb22bb8f5acdc3
        let hash = md5_hash(b"hello world");
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5eb63bbbe01eeed093cb22bb8f5acdc3");
    }

    #[test]
    fn md5_empty() {
        let hash = md5_hash(b"");
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn parse_mapi_result_basic() {
        let lines: Vec<String> = vec![
            "&1 3 2".to_owned(),
            "% id,\tname # name".to_owned(),
            "% int,\tvarchar # type".to_owned(),
            "[ 1,\tAlice ]".to_owned(),
            "[ 2,\tNULL ]".to_owned(),
        ];
        let (cols, rows) = parse_mapi_result(&lines).unwrap();
        assert_eq!(cols, vec!["id", "name"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values[0], Some("1".to_owned()));
        assert_eq!(rows[0].values[1], Some("Alice".to_owned()));
        assert_eq!(rows[1].values[1], None);
    }

    #[test]
    fn parse_mapi_result_error() {
        let lines = vec!["! ERROR: relation not found".to_owned()];
        assert!(parse_mapi_result(&lines).is_err());
    }

    #[test]
    fn inline_params_no_params() {
        assert_eq!(inline_params_monetdb("SELECT 1", &[]), "SELECT 1");
    }

    #[test]
    fn inline_params_quote_escape() {
        let out = inline_params_monetdb("SELECT ?", &["O'Brien".to_owned()]);
        assert_eq!(out, "SELECT 'O''Brien'");
    }

    #[tokio::test]
    async fn stub_no_connection_returns_error() {
        // MonetDB backend is always compiled (not feature-gated), but without
        // a running server the connect() call should fail with an Error::Marshal.
        let mut backend = MonetDbBackend::new("127.0.0.1", 59999, "user", "pass", "db");
        let r = backend.column_names("SELECT 1").await;
        assert!(r.is_err(), "should fail with no server on port 59999");
    }

    /// A local mock MAPI server (real TCP loopback, no live MonetDB needed): sends
    /// the challenge, accepts any auth response, replies READY, then answers the
    /// next query with a canned MAPI result. Exercises `MapiConn::connect` +
    /// `.query()` — i.e. `read_mapi_message`/`send_mapi_message` — end to end over
    /// a real socket, the same framing code a live server would drive.
    fn spawn_mock_mapi_server(result: &'static str) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock mapi server");
        let port = listener.local_addr().expect("local_addr").port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone for read"));
            // Challenge: "server:challenge:protocol:hashes:bigendian:database:"
            send_mapi_message(&stream, b"mockserver:chal:9:MD5,SHA256:0:testdb:\n")
                .expect("send challenge");
            let _auth_response = read_mapi_message(&mut reader).expect("read auth response");
            // Any non-'!'-prefixed reply signals auth success.
            send_mapi_message(&stream, b"\n").expect("send auth ok");
            let _query = read_mapi_message(&mut reader).expect("read query");
            send_mapi_message(&stream, result.as_bytes()).expect("send result");
        });
        port
    }

    #[tokio::test]
    async fn monetdb_column_names_via_mock_mapi_server() {
        let port = spawn_mock_mapi_server("&1 0 1 1\n% n # name\n% int # type\n[ 1\t]\n");
        let mut backend = MonetDbBackend::new("127.0.0.1", port, "monetdb", "monetdb", "testdb");
        let cols = backend
            .column_names("SELECT n FROM t")
            .await
            .expect("column_names over mock server");
        assert_eq!(cols, vec!["n".to_owned()]);
    }

    #[tokio::test]
    async fn monetdb_open_branch_returns_data_rows() {
        let port = spawn_mock_mapi_server(
            "&1 0 2 2\n% id,\tname # name\n% int,\tvarchar # type\n[ 1,\tAlice ]\n[ 2,\tNULL ]\n",
        );
        let mut backend = MonetDbBackend::new("127.0.0.1", port, "monetdb", "monetdb", "testdb");
        let mut stream = backend
            .open_branch("SELECT id, name FROM t", &[])
            .await
            .expect("open_branch over mock server");
        let row1 = stream.next_row().await.unwrap().expect("row 1");
        assert_eq!(
            row1.values,
            vec![Some("1".to_owned()), Some("Alice".to_owned())]
        );
        let row2 = stream.next_row().await.unwrap().expect("row 2");
        assert_eq!(row2.values, vec![Some("2".to_owned()), None]);
        assert!(stream.next_row().await.unwrap().is_none(), "no third row");
    }

    #[tokio::test]
    async fn monetdb_auth_rejected_surfaces_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            send_mapi_message(&stream, b"mockserver:chal:9:MD5,SHA256:0:testdb:\n")
                .expect("send challenge");
            let _auth_response = read_mapi_message(&mut reader).expect("read auth response");
            send_mapi_message(&stream, b"!InvalidCredentialsException:bad login\n")
                .expect("send auth rejection");
        });
        let mut backend = MonetDbBackend::new("127.0.0.1", port, "baduser", "badpass", "testdb");
        let r = backend.column_names("SELECT 1").await;
        assert!(
            r.is_err(),
            "auth rejection ('!' reply) must surface as an error"
        );
    }

    /// `read_mapi_message` must reassemble a message spanning multiple MAPI blocks
    /// (the `last` bit unset on all but the final block) — `send_mapi_message`
    /// always sends a single block, so this drives the wire format directly to
    /// exercise the continuation-loop branch nothing else in this file reaches.
    #[test]
    fn read_mapi_message_reassembles_multiple_blocks() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let writer = std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(addr).expect("connect");
            // Block 1: "hello, " with last_flag=0.
            let part1 = b"hello, ";
            let hdr1: u16 = (part1.len() as u16) << 1; // last bit unset
            stream.write_all(&hdr1.to_le_bytes()).unwrap();
            stream.write_all(part1).unwrap();
            // Block 2: "world" with last_flag=1.
            let part2 = b"world";
            let hdr2: u16 = ((part2.len() as u16) << 1) | 1;
            stream.write_all(&hdr2.to_le_bytes()).unwrap();
            stream.write_all(part2).unwrap();
            stream.flush().unwrap();
        });
        let (server_side, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(server_side);
        let msg = read_mapi_message(&mut reader).expect("reassemble multi-block message");
        assert_eq!(msg, "hello, world");
        writer.join().unwrap();
    }
}
