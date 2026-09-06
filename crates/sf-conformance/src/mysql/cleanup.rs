//! Owned scratch-database cleanup with a bounded drop fallback.

use std::time::Duration;

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts};

const CLEANUP_BUDGET: Duration = Duration::from_secs(5);

pub(super) struct ScratchDatabase {
    admin: Option<Conn>,
    reconnect: Opts,
    name: Option<String>,
}

impl ScratchDatabase {
    pub(super) fn new(admin: Conn, reconnect: Opts, name: String) -> Self {
        Self {
            admin: Some(admin),
            reconnect,
            name: Some(name),
        }
    }

    pub(super) async fn cleanup(mut self) -> Result<(), String> {
        let Some(name) = self.name.clone() else {
            return Ok(());
        };
        let Some(mut admin) = self.admin.take() else {
            return Err("MySQL scratch cleanup has no admin connection".to_owned());
        };
        let completed = tokio::time::timeout(CLEANUP_BUDGET, async move {
            let dropped = admin
                .query_drop(format!("DROP DATABASE IF EXISTS `{name}`"))
                .await
                .map_err(|_| "drop MySQL scratch database failed".to_owned());
            let closed = admin
                .disconnect()
                .await
                .map_err(|_| "close MySQL admin connection failed".to_owned());
            (dropped, closed)
        })
        .await;
        match completed {
            Err(_) => Err("MySQL scratch database cleanup timed out".to_owned()),
            Ok((Err(error), _)) => Err(error),
            Ok((Ok(()), Err(error))) => {
                self.name = None;
                Err(error)
            }
            Ok((Ok(()), Ok(()))) => {
                self.name = None;
                Ok(())
            }
        }
    }
}

impl Drop for ScratchDatabase {
    fn drop(&mut self) {
        let Some(name) = self.name.take() else {
            return;
        };
        let opts = self.reconnect.clone();
        // A dropped/cancelled async future cannot await cleanup. Reconnect on a
        // private runtime and wait only for one fixed budget; never borrow the
        // original runtime or leak credentials through diagnostics.
        let fallback = std::thread::Builder::new()
            .name("sf-mysql-scratch-cleanup".to_owned())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(async move {
                    let _ = tokio::time::timeout(CLEANUP_BUDGET, async move {
                        let Ok(mut admin) = Conn::new(opts).await else {
                            return;
                        };
                        let _ = admin
                            .query_drop(format!("DROP DATABASE IF EXISTS `{name}`"))
                            .await;
                        let _ = admin.disconnect().await;
                    })
                    .await;
                });
            });
        if let Ok(fallback) = fallback {
            let _ = fallback.join();
        }
    }
}
