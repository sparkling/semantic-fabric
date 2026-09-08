//! Explicit Rust parser host for programmatic serving embeddings.
//! No ordinary command or environment-selected parser entry is accepted.
fn main() {
    sf_sparql::dispatch_private_parser_worker_v1();
    std::process::exit(78);
}
