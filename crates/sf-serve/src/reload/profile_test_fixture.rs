use super::*;

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) opts: Arc<ServeOptions>,
    pub(super) source: PreparedSource,
    pub(super) baseline: std::sync::Mutex<Option<Arc<Baseline>>>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self::with_profile(crate::QueryShapeProfile::Ordinary)
    }

    pub(super) fn with_profile(profile: crate::QueryShapeProfile) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sf-reload-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let db = root.join("source.db");
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE items(value TEXT); INSERT INTO items VALUES ('old');")
            .unwrap();
        std::fs::write(
            root.join("ontology.ttl"),
            "<http://example.test/value> a <http://www.w3.org/1999/02/22-rdf-syntax-ns#Property> .",
        )
        .unwrap();
        std::fs::write(root.join("mapping.ttl"), mapping("rr:column \"value\"")).unwrap();
        let opts = Arc::new(ServeOptions {
            query_shape_profile: profile,
            query_admission: crate::QueryAdmission::Bearer(
                crate::BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
            ),
            source: crate::SourceRef::inline(format!("sqlite:{}", db.display())),
            mapping: crate::MappingRef::r2rml_file(root.join("mapping.ttl").to_str().unwrap()),
            additional_source: None,
            ontology_path: root.join("ontology.ttl").to_string_lossy().into_owned(),
            bind: "127.0.0.1:0".into(),
            timeout: Duration::from_secs(5),
            max_query_len: 4096,
            max_concurrent_requests: 8,
            // These cases qualify reload ownership, not an isolated work phase.
            // Keep complete request preparation funded at the application default.
            max_compiler_work: crate::DEFAULT_QUERY_LIMITS.max_compiler_work(),
            max_source_work: crate::DEFAULT_QUERY_LIMITS.max_source_work(),
            max_result_items: 1000,
            max_order_rows: 100,
            max_order_bytes: 4096,
            max_serialized_bytes: 4096,
            pg_pool_size: 1,
            pg_pool_wait: Duration::from_secs(1),
            sqlite_pool_size: 1,
            shutdown_timeout: Duration::from_secs(1),
            reload_interval: Duration::from_secs(1),
            require_verified_generation: false,
            metrics: None,
        });
        let source = opts.source.resolve().unwrap().prepare().unwrap();
        Self {
            root,
            opts,
            source,
            baseline: std::sync::Mutex::new(None),
        }
    }

    pub(super) async fn config(&self) -> Arc<ServeConfig> {
        let (mut config, baseline) =
            crate::startup::build_config(&self.opts, self.source.clone(), None)
                .await
                .unwrap();
        // These unit fixtures exercise generation ownership, not the separately
        // qualified CLI parser process. Never enable an implicit runtime fallback.
        config.use_in_process_test_parser();
        *self.baseline.lock().unwrap() = Some(Arc::new(baseline));
        Arc::new(config)
    }

    pub(super) async fn refresh(&self, config: &ServeConfig) {
        let runtime = config.lifecycle_runtime();
        let expected = runtime.readiness().unwrap();
        let baseline = self.baseline.lock().unwrap().as_ref().unwrap().clone();
        if let Some(next) = refresh(
            runtime,
            baseline,
            Arc::clone(&self.opts),
            self.source.clone(),
            None,
            expected,
            None,
        )
        .await
        {
            *self.baseline.lock().unwrap() = Some(next);
        }
    }

    pub(super) fn replace_mapping(&self, object: &str) {
        std::fs::write(self.root.join("mapping.ttl"), mapping(object)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // This directory was created exclusively by this fixture, never user input.
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
