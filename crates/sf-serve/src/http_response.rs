//! Backend response execution after request and shape admission.
use super::*;

/// Stream a SELECT (ADR-0010 §C). The status line is committed once streaming
/// begins, so the recoverable errors (parse → 400, unsupported → 501) are already
/// resolved by [`compile`]; an execution failure or a passed deadline errors the
/// body mid-stream (same posture as the SQLite CONSTRUCT path). HTTP 200 is already
/// committed then, so this slice does not claim an atomic/no-prefix result body.
pub(super) async fn respond_select(
    backend: Backend,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    rls_tables: Option<Arc<[String]>>,
    accept: Option<&str>,
    budget: RequestBudget,
) -> Response {
    let fmt = negotiate_results(accept);
    let PlanForm::Select { vars } = &plan.form else {
        return problem::response(ProblemCode::Internal);
    };
    let vars = vars.clone();
    let body = match backend {
        Backend::Sqlite(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            let lease = match sqlite_admission::acquire(&pool, &budget).await {
                Ok(lease) => lease,
                Err(response) => return response,
            };
            let drive_control: Arc<dyn QueryControl> = Arc::new(budget.clone());
            stream::select_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec::select_each_sqlite_owned_interruptible_leased(
                            &plan,
                            lease,
                            drive_control,
                            sink,
                        )
                        .await
                    })
                },
                fmt,
                vars,
                budget,
            )
        }
        Backend::Pg(pool) => {
            match crate::pg_response::select(pool, plan, generation, rls_tables, fmt, vars, budget)
                .await
            {
                Ok(body) => body,
                Err(response) => return response,
            }
        }
        Backend::Mysql(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            let conn = match crate::source_acquisition::acquire_mysql(&pool, &budget).await {
                Ok(conn) => conn,
                Err(response) => return response,
            };
            let drive_budget = budget.clone();
            stream::select_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec_mysql::select_each_mysql_controlled(&plan, conn, &drive_budget, sink)
                            .await
                    })
                },
                fmt,
                vars,
                budget,
            )
        }
    };
    ok_stream(fmt.media_type(), body)
}

pub(super) async fn respond_ask(
    backend: Backend,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    rls_tables: Option<Arc<[String]>>,
    accept: Option<&str>,
    budget: RequestBudget,
) -> Response {
    if let Err(error) = budget.preflight_ask_result() {
        return problem::response_for_control(error);
    }
    let fmt = negotiate_results(accept);
    let value = match backend {
        Backend::Sqlite(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            let lease = match sqlite_admission::acquire(&pool, &budget).await {
                Ok(lease) => lease,
                Err(response) => return response,
            };
            let task_control: Arc<dyn QueryControl> = Arc::new(budget.clone());
            let run = deadline::spawn_request_task(async move {
                exec::ask_sqlite_owned_interruptible_leased(&plan, lease, task_control).await
            });
            match deadline::join_task(budget.clone(), run).await {
                Err(JoinedTaskError::Control(error)) => {
                    return problem::response_for_control(error)
                }
                Err(JoinedTaskError::Join(_)) => return problem::response(ProblemCode::Internal),
                Ok(r) => r,
            }
        }
        Backend::Pg(pool) => {
            match crate::pg_response::ask(pool, plan, generation, rls_tables, budget.clone()).await
            {
                Ok(result) => result,
                Err(response) => return response,
            }
        }
        Backend::Mysql(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            // ASK collects (a single boolean). Unlike PG (whose `PgRowStream` is
            // `'static`), MySQL's branch cursor BORROWS the connection, so awaiting
            // `ask_mysql` inline in this handler future leaves the borrowing stream
            // held across an await — an HRTB `Send` obligation axum's handler future
            // cannot discharge. `spawn_request_task` proves `Send` on the concrete
            // owned-`Conn` task, dropped/disposed after the run (§4.2), mirroring
            // the SQLite ASK arm's `tokio::spawn` + `Ok(Err)/Ok(Ok)` handling.
            let conn = match crate::source_acquisition::acquire_mysql(&pool, &budget).await {
                Ok(conn) => conn,
                Err(response) => return response,
            };
            let task_budget = budget.clone();
            let run = deadline::spawn_request_task(async move {
                exec_mysql::ask_each_mysql_controlled(&plan, conn, &task_budget).await
            });
            match deadline::join_task(budget.clone(), run).await {
                Err(JoinedTaskError::Control(error)) => {
                    return problem::response_for_control(error)
                }
                Err(JoinedTaskError::Join(_)) => return problem::response(ProblemCode::Internal),
                Ok(r) => r,
            }
        }
    };
    match value {
        Ok(b) => {
            if let Err(error) = budget.checkpoint() {
                return problem::response_for_control(error);
            }
            match traced_sync(Stage::Serialize, || stream::serialize_boolean(b, fmt)) {
                Ok(bytes) => {
                    let Ok(amount) = u64::try_from(bytes.len()) else {
                        return problem::response(ProblemCode::Internal);
                    };
                    if let Err(error) = budget.consume(QueryCharge::SerializedBytes, amount) {
                        return problem::response_for_control(error);
                    }
                    if let Err(error) = budget.checkpoint() {
                        return problem::response_for_control(error);
                    }
                    ok_stream(fmt.media_type(), stream::collected_body(bytes))
                }
                Err(_) => problem::response(ProblemCode::Internal),
            }
        }
        Err(e) => problem::response_for_sparql(&e),
    }
}

/// Stream a CONSTRUCT (ADR-0010 §C) — triples flow from the executor sink through
/// the RDF serialiser into the body, never collected, on **both** backends.
pub(super) async fn respond_construct(
    backend: Backend,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    rls_tables: Option<Arc<[String]>>,
    accept: Option<&str>,
    budget: RequestBudget,
) -> Response {
    let fmt = negotiate_rdf(accept);
    let body = match backend {
        Backend::Sqlite(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            let lease = match sqlite_admission::acquire(&pool, &budget).await {
                Ok(lease) => lease,
                Err(response) => return response,
            };
            let drive_control: Arc<dyn QueryControl> = Arc::new(budget.clone());
            stream::construct_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec::construct_each_sqlite_owned_interruptible_leased(
                            &plan,
                            lease,
                            drive_control,
                            sink,
                        )
                        .await
                    })
                },
                fmt,
                budget,
            )
        }
        Backend::Pg(pool) => {
            match crate::pg_response::construct(pool, plan, generation, rls_tables, fmt, budget)
                .await
            {
                Ok(body) => body,
                Err(response) => return response,
            }
        }
        Backend::Mysql(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            let conn = match crate::source_acquisition::acquire_mysql(&pool, &budget).await {
                Ok(conn) => conn,
                Err(response) => return response,
            };
            let drive_budget = budget.clone();
            stream::construct_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec_mysql::construct_each_mysql_controlled(
                            &plan,
                            conn,
                            &drive_budget,
                            sink,
                        )
                        .await
                    })
                },
                fmt,
                budget,
            )
        }
    };
    ok_stream(fmt.media_type(), body)
}
