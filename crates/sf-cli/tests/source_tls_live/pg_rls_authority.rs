//! Native prerequisites, not admission of protected source-RLS generations.
use super::*;
#[path = "pg_session.rs"]
mod pg_session;
use pg_session::Session;

const SELECT: &str = "SELECT value FROM public.items ORDER BY value";
const PREDICATE: &str = "tenant = current_setting('app.tenant_id', true)";

fn begin(reader: &mut Session) -> i32 {
    let rows = reader.stage(
        "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; \
        SET LOCAL statement_timeout='5s'; SET LOCAL lock_timeout='1s'; \
        SET LOCAL app.tenant_id='A'; SET LOCAL plan_cache_mode=force_generic_plan; \
        LOCK TABLE public.items IN ACCESS SHARE MODE; SELECT pg_backend_pid()",
    );
    assert_eq!(rows.len(), 1);
    rows[0].parse().unwrap()
}

fn held(database: &Database, pid: i32) {
    assert_eq!(
        database.sql(&format!(
            "SELECT count(*) FROM pg_locks WHERE pid={pid} \
        AND relation='public.items'::regclass AND mode='AccessShareLock' AND granted"
        )),
        "1"
    );
    assert_eq!(
        database.sql(&format!("SELECT ssl FROM pg_stat_ssl WHERE pid={pid}")),
        "t"
    );
}

fn policy_locks(database: &Database) {
    let original = format!("CREATE POLICY identity ON public.items USING ({PREDICATE})");
    for (change, restore) in [
        (
            "ALTER POLICY identity ON public.items USING (false)".to_owned(),
            format!("ALTER POLICY identity ON public.items USING ({PREDICATE})"),
        ),
        ("DROP POLICY identity ON public.items".into(), original),
        (
            "CREATE POLICY extra ON public.items USING (true)".into(),
            "DROP POLICY extra ON public.items".into(),
        ),
        (
            "ALTER TABLE public.items DISABLE ROW LEVEL SECURITY".into(),
            "ALTER TABLE public.items ENABLE ROW LEVEL SECURITY".into(),
        ),
        (
            "ALTER TABLE public.items NO FORCE ROW LEVEL SECURITY".into(),
            "ALTER TABLE public.items FORCE ROW LEVEL SECURITY".into(),
        ),
    ] {
        let mut reader = Session::new(database);
        let pid = begin(&mut reader);
        assert_eq!(reader.stage(SELECT), ["a1", "a2"]);
        held(database, pid);
        // Catch only native lock_timeout. Unexpected success or another error
        // fails the fixture; no sleep is used as proof of exclusion.
        database.sql(&format!(
            "SET lock_timeout='100ms'; DO $probe$ BEGIN \
            EXECUTE $ddl${change}$ddl$; RAISE EXCEPTION 'policy DDL crossed held lease'; \
            EXCEPTION WHEN lock_not_available THEN NULL; END $probe$"
        ));
        held(database, pid);
        reader.stage("ROLLBACK");
        database.sql(&format!("SET lock_timeout='1s'; {change}; {restore}"));
    }
}

fn capability_change(database: &Database, patch: &str, capability: &str, initially_bypass: bool) {
    let (grant, revoke) = match capability {
        "bypass" => (
            "ALTER ROLE sf_tls BYPASSRLS",
            "ALTER ROLE sf_tls NOBYPASSRLS",
        ),
        "superuser" => (
            "ALTER ROLE sf_tls SUPERUSER",
            "ALTER ROLE sf_tls NOSUPERUSER",
        ),
        _ => unreachable!(),
    };
    database.sql(if initially_bypass { grant } else { revoke });
    let mut reader = Session::new(database);
    let pid = begin(&mut reader);
    let role_fact = match capability {
        "bypass" => "SELECT rolbypassrls FROM pg_roles WHERE rolname='sf_tls'",
        "superuser" => "SELECT rolsuper FROM pg_roles WHERE rolname='sf_tls'",
        _ => unreachable!(),
    };
    let expected_fact = if initially_bypass { "t" } else { "f" };
    assert_eq!(reader.stage(role_fact), [expected_fact]);
    let before = if initially_bypass {
        vec!["a1", "a2", "b1"]
    } else {
        vec!["a1", "a2"]
    };
    let after = if initially_bypass {
        vec!["a1", "a2"]
    } else {
        vec!["a1", "a2", "b1"]
    };
    assert_eq!(
        reader.stage(&format!("PREPARE saved AS {SELECT}; EXECUTE saved")),
        before
    );
    assert_eq!(
        reader.stage(&format!(
            "DECLARE retained NO SCROLL CURSOR FOR {SELECT}; FETCH 1 FROM retained"
        )),
        ["a1"]
    );
    held(database, pid);
    database.sql(&format!(
        "SET lock_timeout='1s'; {}",
        if initially_bypass { revoke } else { grant }
    ));
    held(database, pid);
    // A catalogue recheck retains the RR snapshot even though the role cache
    // and a newly planned statement already use the changed authority.
    assert_eq!(reader.stage(role_fact), [expected_fact]);
    assert_eq!(
        reader.stage("SELECT row_security_active('public.items'::regclass)"),
        [if initially_bypass { "t" } else { "f" }]
    );
    assert_eq!(reader.stage("FETCH ALL FROM retained"), before[1..]);
    assert_eq!(
        reader.stage(SELECT),
        after,
        "new statement: {patch}/{capability}"
    );
    let cached = reader.stage("EXECUTE saved");
    eprintln!("{patch}/{capability}/initially_bypass={initially_bypass}: cached={cached:?}");
    assert_eq!(cached, if patch == "16.9" { before } else { after });
    reader.stage("ROLLBACK");
    database.sql(revoke);
}

fn membership_change(database: &Database, patch: &str, initially_member: bool) {
    let grant = "GRANT sf_policy_reader TO sf_tls WITH INHERIT TRUE";
    let revoke = "REVOKE sf_policy_reader FROM sf_tls";
    database.sql(if initially_member { grant } else { revoke });
    let mut reader = Session::new(database);
    let pid = begin(&mut reader);
    let before = if initially_member {
        vec!["a1", "a2", "b1"]
    } else {
        vec!["a1", "a2"]
    };
    let after = if initially_member {
        vec!["a1", "a2"]
    } else {
        vec!["a1", "a2", "b1"]
    };
    assert_eq!(
        reader.stage(&format!("PREPARE saved AS {SELECT}; EXECUTE saved")),
        before
    );
    assert_eq!(
        reader.stage(&format!(
            "DECLARE retained NO SCROLL CURSOR FOR {SELECT}; FETCH 1 FROM retained"
        )),
        ["a1"]
    );
    database.sql(&format!(
        "SET lock_timeout='1s'; {}",
        if initially_member { revoke } else { grant }
    ));
    held(database, pid);
    assert_eq!(reader.stage("FETCH ALL FROM retained"), before[1..]);
    assert_eq!(reader.stage(SELECT), after);
    let cached = reader.stage("EXECUTE saved");
    eprintln!("{patch}/membership/initially_member={initially_member}: cached={cached:?}");
    assert_eq!(cached, if patch == "16.9" { before } else { after });
    reader.stage("ROLLBACK");
    database.sql(revoke);
}

#[test]
#[ignore = "requires Docker and both pinned owned PostgreSQL images; required in CI"]
fn policy_locks_and_role_changes_have_distinct_authority() {
    for patch in ["16.9", "16.15"] {
        let fixture = Fixture::new();
        let database = Database::postgres_patch(&fixture, patch);
        assert_eq!(
            database.sql("SHOW server_version_num"),
            if patch == "16.9" { "160009" } else { "160015" }
        );
        database.sql(&format!("ALTER TABLE public.items ADD COLUMN tenant text NOT NULL DEFAULT 'A'; \
            DELETE FROM public.items; INSERT INTO public.items VALUES ('a1','A'),('a2','A'),('b1','B'); \
            ALTER TABLE public.items ENABLE ROW LEVEL SECURITY; ALTER TABLE public.items FORCE ROW LEVEL SECURITY; \
            CREATE POLICY identity ON public.items USING ({PREDICATE}); CREATE ROLE sf_policy_reader NOLOGIN"));
        policy_locks(&database);
        for initially_bypass in [false, true] {
            for capability in ["bypass", "superuser"] {
                capability_change(&database, patch, capability, initially_bypass);
            }
        }
        database.sql("CREATE POLICY membership ON public.items TO sf_policy_reader USING (true)");
        for initially_member in [false, true] {
            membership_change(&database, patch, initially_member);
        }
    }
}
