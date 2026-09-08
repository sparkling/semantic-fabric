//! Explicit parser host for public-router conformance fixtures only.
fn main() {
    sf_sparql::dispatch_private_parser_worker_v1();
    std::process::exit(78);
}
