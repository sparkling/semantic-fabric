use sf_bench::performance::profile::{
    parse_profile, render_profile, RunnerProbe, RunnerProfile, RunnerSnapshot,
};

#[derive(Clone)]
struct FakeProbe(RunnerSnapshot);

impl RunnerProbe for FakeProbe {
    type Error = std::convert::Infallible;

    fn probe(&self) -> Result<RunnerSnapshot, Self::Error> {
        Ok(self.0.clone())
    }
}

fn snapshot() -> RunnerSnapshot {
    RunnerSnapshot {
        os: "linux".into(),
        architecture: "x86_64".into(),
        kernel_release: "6.12.1-controlled".into(),
        cpu_model: "Synthetic CPU".into(),
        online_cpus: "0-7".into(),
        allowed_cpus: "6-7".into(),
        isolated_cpus: "6-7".into(),
        scaling_governor: "performance".into(),
        turbo: "disabled".into(),
        swap_total_kib: 0,
        mem_total_kib: 67_108_864,
        load1_milli: 100,
        build_profile: "release".into(),
    }
}

fn profile() -> RunnerProfile {
    let snapshot = snapshot();
    RunnerProfile {
        profile_id: "controlled-linux-test-v1".into(),
        controlled: true,
        os: snapshot.os,
        architecture: snapshot.architecture,
        kernel_release: snapshot.kernel_release,
        cpu_model: snapshot.cpu_model,
        online_cpus: snapshot.online_cpus,
        allowed_cpus: snapshot.allowed_cpus,
        isolated_cpus: snapshot.isolated_cpus,
        scaling_governor: snapshot.scaling_governor,
        turbo: snapshot.turbo,
        swap_total_kib: snapshot.swap_total_kib,
        mem_total_kib: snapshot.mem_total_kib,
        load1_limit_milli: 250,
        build_profile: snapshot.build_profile,
    }
}

#[test]
fn should_round_trip_and_validate_an_exact_controlled_profile() {
    let expected = profile();
    let text = render_profile(&expected).unwrap();
    let parsed = parse_profile(text.as_bytes()).unwrap();

    parsed.validate(&FakeProbe(snapshot())).unwrap();

    assert_eq!(parsed, expected);
    assert_eq!(parsed.digest().unwrap().len(), 64);
}

#[test]
fn should_refuse_an_explicitly_uncontrolled_profile() {
    let mut expected = profile();
    expected.controlled = false;

    assert!(expected.validate(&FakeProbe(snapshot())).is_err());
}

#[test]
fn should_refuse_static_profile_drift_and_excess_load() {
    let expected = profile();
    let mut drifted = snapshot();
    drifted.kernel_release = "6.12.2".into();
    assert!(expected.validate(&FakeProbe(drifted)).is_err());

    let mut loaded = snapshot();
    loaded.load1_milli = 251;
    assert!(expected.validate(&FakeProbe(loaded)).is_err());
}

#[test]
fn should_require_the_allowed_cpuset_to_be_isolated() {
    let expected = profile();
    let mut shared = snapshot();
    shared.isolated_cpus = "7".into();

    assert!(expected.validate(&FakeProbe(shared)).is_err());
}

#[test]
fn should_require_the_allowed_cpuset_to_be_online() {
    let expected = profile();
    let mut offline = snapshot();
    offline.online_cpus = "0-6".into();

    assert!(expected.validate(&FakeProbe(offline)).is_err());
}
