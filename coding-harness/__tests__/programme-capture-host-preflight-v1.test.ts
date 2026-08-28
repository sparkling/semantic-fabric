// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  collectProgrammeCaptureHostDiagnosticObservationV1,
  collectProgrammeCaptureHostObservationV1,
  diagnoseProgrammeCaptureHostObservationV1,
  parseProgrammeCaptureHostNonAdmissionV1,
  rejectProgrammeCaptureHostPreflightV1,
  rustArchitecture,
  verifyProgrammeCaptureHostNonAdmissionV1,
  type ProgrammeCaptureHostSurfaceSourceV1,
} from '../src/programme-capture-host-preflight-v1.js';
import {
  parseProgrammeCaptureRunnerProfileV1,
  renderProgrammeCaptureRunnerProfileV1,
} from '../src/programme-capture-runner-profile-v1.js';
import {
  parseProgrammeCaptureInputAttestationV1,
  type ProgrammeCaptureInputAttestationV1,
} from '../src/programme-capture-input-attestation-v1.js';
import {
  createProgrammeCaptureStateV1,
  transitionProgrammeCaptureStateV1,
} from '../src/programme-capture-state-v1.js';
import {
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  PROGRAMME_CAPTURE_SCENARIOS_PATH,
} from '../src/programme-capture-task-v1.js';
import { digestValue } from '../src/receipts.js';

const sha256 = (value: string | Uint8Array): string =>
  createHash('sha256').update(value).digest('hex');
const digest = (character: string): string => character.repeat(64);

const PROFILE_TEXT = [
  'sf-performance-runner-profile-v1',
  'profile-id\tcontrolled-linux-test-v1',
  'controlled\ttrue',
  'os\tlinux',
  'architecture\tx86_64',
  'kernel-release\t6.12.1-controlled',
  'cpu-model\tSynthetic CPU',
  'online-cpus\t0-7',
  'allowed-cpus\t6-7',
  'isolated-cpus\t6-7',
  'scaling-governor\tperformance',
  'turbo\tdisabled',
  'swap-total-kib\t0',
  'mem-total-kib\t67108864',
  'load1-limit-milli\t250',
  'build-profile\trelease',
  '',
].join('\n');

class FakeSurfaceSource implements ProgrammeCaptureHostSurfaceSourceV1 {
  readonly platform = 'linux';
  readonly architecture = 'x86_64';
  readonly reads: string[] = [];
  readonly #values: Map<string, string>;
  #tick = 0;

  constructor(overrides: Readonly<Record<string, string | null>> = {}) {
    this.#values = new Map(Object.entries({
      '/proc/sys/kernel/osrelease': '6.12.1-controlled\n',
      '/proc/cpuinfo': [
        'processor\t: 0',
        'model name\t: Synthetic CPU',
        'microcode\t: 0x42',
        '',
      ].join('\n'),
      '/proc/self/status': [
        'Name:\tnode',
        'Cpus_allowed_list:\t6-7',
        'Mems_allowed_list:\t0',
        '',
      ].join('\n'),
      '/proc/meminfo': 'MemTotal:       67108864 kB\nSwapTotal:             0 kB\n',
      '/proc/loadavg': '0.10 0.20 0.30 1/100 42\n',
      '/proc/self/cgroup': '0::/user.slice/test.scope\n',
      '/sys/devices/system/cpu/online': '0-7\n',
      '/sys/devices/system/cpu/isolated': '6-7\n',
      '/sys/devices/system/cpu/cpu6/cpufreq/scaling_governor': 'performance\n',
      '/sys/devices/system/cpu/cpu7/cpufreq/scaling_governor': 'performance\n',
      '/sys/devices/system/cpu/cpufreq/boost': '0\n',
      ...overrides,
    }).filter((entry): entry is [string, string] => entry[1] !== null));
  }

  now(): string {
    this.#tick += 1;
    return `2026-08-28T12:00:0${this.#tick}.000Z`;
  }

  read(path: string, maximumBytes: number): Uint8Array {
    this.reads.push(path);
    const value = this.#values.get(path);
    if (value === undefined) {
      const error = new Error('missing') as NodeJS.ErrnoException;
      error.code = 'ENOENT';
      throw error;
    }
    const bytes = Buffer.from(value);
    if (bytes.length > maximumBytes) throw new Error('HARNESS_CAPTURE_HOST_SURFACE_OVERSIZE');
    return bytes;
  }
}

function inputAttestation(profileBytes = Buffer.from(PROFILE_TEXT)):
ProgrammeCaptureInputAttestationV1 {
  const controller = { commit: 'c'.repeat(40), tree: 'd'.repeat(40) };
  const paths = [
    PROGRAMME_CAPTURE_PROFILE_PATH,
    PROGRAMME_CAPTURE_SCENARIOS_PATH,
    'Cargo.lock',
    ...PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  ];
  const protectedInputs = paths.map((path, index) => ({
    path,
    gitBlobId: index === 0 ? '1'.repeat(40) : '2'.repeat(40),
    sha256: index === 0 ? sha256(profileBytes) : digest('a'),
    byteLength: index === 0 ? profileBytes.length : 1,
  }));
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    controller,
    manifest: {
      path: 'coding-harness/.harness/manifest.json',
      gitBlobId: '3'.repeat(40),
      sha256: digest('b'),
      byteLength: 1,
    },
    task: {
      path: 'coding-harness/config/m0-performance-baseline-acceptance.json',
      gitBlobId: '4'.repeat(40),
      sha256: digest('c'),
      byteLength: 1,
      valueDigest: digest('d'),
    },
    protectedInputs,
    output: {
      path: 'crates/sf-bench/config/performance-baseline-v1.tsv' as const,
      absentFromCommit: true as const,
    },
    protectedInputsDigest: digestValue(protectedInputs),
  };
  return parseProgrammeCaptureInputAttestationV1({
    ...body,
    attestationDigest: digestValue(body),
  });
}

function inputsAttestedState(attestation: ProgrammeCaptureInputAttestationV1) {
  const admitted = createProgrammeCaptureStateV1({
    runId: 'capture_run_20260828_0001',
    taskDigest: attestation.task.valueDigest,
    claimDigest: digest('e'),
    controller: attestation.controller,
    admissionEvidenceDigest: digest('f'),
  });
  return transitionProgrammeCaptureStateV1(admitted, {
    kind: 'attest-inputs',
    evidenceDigest: attestation.attestationDigest,
  });
}

function rejectionFixture(
  source = new FakeSurfaceSource(),
  profileBytes: Uint8Array | null = Buffer.from(PROFILE_TEXT),
) {
  const attestation = inputAttestation();
  return rejectProgrammeCaptureHostPreflightV1({
    state: inputsAttestedState(attestation),
    inputAttestation: attestation,
    profileBytes: profileBytes ?? undefined,
    observation: collectProgrammeCaptureHostDiagnosticObservationV1(source),
  });
}

describe('programme capture negative-only host preflight V1', () => {
  it('accepts a conservative canonical subset of the Rust profile without making it authority', () => {
    const parsed = parseProgrammeCaptureRunnerProfileV1(Buffer.from(PROFILE_TEXT));
    expect(renderProgrammeCaptureRunnerProfileV1(parsed)).toBe(PROFILE_TEXT);
    expect(parsed).toMatchObject({
      profileId: 'controlled-linux-test-v1',
      controlled: true,
      allowedCpus: '6-7',
      load1LimitMilli: 250,
    });
    expect(Object.isFrozen(parsed)).toBe(true);

    for (const mutate of [
      (text: string) => text.replace('controlled\ttrue', 'controlled\tTRUE'),
      (text: string) => text.replace('swap-total-kib\t0', 'swap-total-kib\t00'),
      (text: string) => text.replace('profile-id\t', 'profile-id\tx\t'),
      (text: string) => text.replace(/\n$/, ''),
      (text: string) => text.replace(/\n/g, '\r\n'),
      (text: string) => text.replace('sf-performance-runner-profile-v1', 'legacy-profile'),
      (text: string) => `${text}extra\tfield\n`,
    ]) {
      expect(() => parseProgrammeCaptureRunnerProfileV1(Buffer.from(mutate(PROFILE_TEXT))))
        .toThrow();
    }
    expect(() => parseProgrammeCaptureRunnerProfileV1(Buffer.alloc(65_537, 97)))
      .toThrow(/byte bound/);
    expect(() => parseProgrammeCaptureRunnerProfileV1(Uint8Array.from([0xff])))
      .toThrow(/UTF-8/);
    expect(() => parseProgrammeCaptureRunnerProfileV1(Buffer.from(
      PROFILE_TEXT.replace('Synthetic CPU', 'é'.repeat(300)),
    ))).toThrow(/cpu model/);
    expect(() => parseProgrammeCaptureRunnerProfileV1(Buffer.from(
      PROFILE_TEXT.replace('mem-total-kib\t67108864', 'mem-total-kib\t18446744073709551615'),
    ))).toThrow(/mem-total-kib/);
  });

  it('never turns an all-green legacy profile into positive admission', () => {
    const result = rejectionFixture();

    expect(result.record.outcome).toBe('unproven');
    expect(result.record.reasons).toEqual(['positive-control-closure-incomplete']);
    expect(result.record.authority).toBe('diagnostic-classified-non-admission');
    expect(result.record.captureAuthorized).toBe(false);
    expect(result.state.phase).toBe('failed');
    expect(result.state.captureAttempts).toBe(0);
    expect(result.state.events.at(-1)).toMatchObject({
      kind: 'fail',
      evidenceDigest: result.record.recordDigest,
    });
  });

  it('records stable absolute disqualifiers and still performs no attempt', () => {
    const source = new FakeSurfaceSource({
      '/proc/self/status': 'Cpus_allowed_list:\t0-7\nMems_allowed_list:\t0\n',
      '/proc/meminfo': 'MemTotal: 67108864 kB\nSwapTotal: 1024 kB\n',
      '/sys/devices/system/cpu/isolated': '',
      '/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpu1/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpu2/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpu3/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpu4/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpu5/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpu6/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpu7/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpufreq/boost': '1\n',
    });
    const result = rejectionFixture(source);

    expect(result.record.outcome).toBe('ineligible');
    expect(result.record.reasons).toEqual([
      'profile-static-mismatch',
      'allowed-cpus-not-isolated',
      'governor-not-performance',
      'turbo-not-disabled',
      'swap-enabled',
    ]);
    expect(result.state.captureAttempts).toBe(0);
    expect(rejectionFixture(new FakeSurfaceSource({
      '/proc/self/status': 'Cpus_allowed_list:\t\nMems_allowed_list:\t0\n',
    })).record.reasons).toContain('allowed-cpus-empty');
    const independent = rejectionFixture(new FakeSurfaceSource({
      '/sys/devices/system/cpu/online': 'invalid\n',
      '/sys/devices/system/cpu/isolated': '0-5\n',
      '/sys/devices/system/cpu/cpu6/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpu7/cpufreq/scaling_governor': 'powersave\n',
      '/sys/devices/system/cpu/cpufreq/boost': '1\n',
      '/proc/meminfo': 'MemTotal: 67108864 kB\nSwapTotal: 1024 kB\n',
    })).record.reasons;
    expect(independent).toEqual([
      'profile-static-mismatch', 'cpu-list-invalid', 'allowed-cpus-not-isolated',
      'governor-not-performance', 'turbo-not-disabled', 'swap-enabled',
    ]);
  });

  it('fails closed on absent, substituted, invalid, incomplete, or drifting evidence', () => {
    expect(rejectionFixture(new FakeSurfaceSource(), null).record).toMatchObject({
      outcome: 'unproven',
      reasons: ['profile-authority-bytes-unavailable'],
    });
    expect(rejectionFixture(
      new FakeSurfaceSource(), Buffer.from(PROFILE_TEXT.replace('Synthetic CPU', 'Other CPU')),
    ).record.reasons).toEqual(['profile-authority-mismatch']);
    expect(rejectionFixture(
      new FakeSurfaceSource(), Buffer.from(PROFILE_TEXT.replace('controlled\ttrue', 'bad\ttrue')),
    ).record.reasons).toEqual(['profile-authority-mismatch']);

    const invalidProfile = Buffer.from(PROFILE_TEXT.replace('controlled\ttrue', 'controlled\tTRUE'));
    const invalidAttestation = inputAttestation(invalidProfile);
    const invalid = rejectProgrammeCaptureHostPreflightV1({
      state: inputsAttestedState(invalidAttestation),
      inputAttestation: invalidAttestation,
      profileBytes: invalidProfile,
      observation: collectProgrammeCaptureHostDiagnosticObservationV1(new FakeSurfaceSource()),
    });
    expect(invalid.record.reasons).toEqual(['profile-invalid']);
    expect(invalid.state.captureAttempts).toBe(0);

    const missing = rejectionFixture(new FakeSurfaceSource({
      '/proc/meminfo': null,
    }));
    expect(missing.record.outcome).toBe('unproven');
    expect(missing.record.reasons).toContain('required-observation-unavailable');

    let statusReads = 0;
    const changing = new FakeSurfaceSource();
    const originalRead = changing.read.bind(changing);
    changing.read = (path, limit) => {
      if (path === '/proc/self/status') {
        statusReads += 1;
        if (statusReads > 1) {
          return Buffer.from('Cpus_allowed_list:\t0-7\nMems_allowed_list:\t0\n');
        }
      }
      return originalRead(path, limit);
    };
    const drifted = rejectionFixture(changing);
    expect(drifted.record.reasons).toContain('observation-changed');
    expect(drifted.state.captureAttempts).toBe(0);

    const emptyAttestation = inputAttestation(Buffer.alloc(0));
    const empty = rejectProgrammeCaptureHostPreflightV1({
      state: inputsAttestedState(emptyAttestation),
      inputAttestation: emptyAttestation,
      profileBytes: Buffer.alloc(0),
      observation: collectProgrammeCaptureHostDiagnosticObservationV1(new FakeSurfaceSource()),
    });
    expect(empty.record.reasons).toEqual(['profile-invalid']);
    expect(empty.state).toMatchObject({ phase: 'failed', captureAttempts: 0 });
  });

  it('separates semantic control drift from raw-source churn and fails closed on turbo denial', () => {
    let cpuReads = 0;
    const noisy = new FakeSurfaceSource(), originalRead = noisy.read.bind(noisy);
    noisy.read = (path, limit) => {
      const bytes = originalRead(path, limit);
      return path === '/proc/cpuinfo'
        ? Buffer.concat([bytes, Buffer.from(`cpu MHz\t: ${++cpuReads}\n`)]) : bytes;
    };
    const stable = collectProgrammeCaptureHostDiagnosticObservationV1(noisy);
    expect(stable.samples[0].snapshotDigest).not.toBe(stable.samples[1].snapshotDigest);
    expect(stable.samples[0].controlDigest).toBe(stable.samples[1].controlDigest);
    expect(diagnoseProgrammeCaptureHostObservationV1(stable).reasons)
      .not.toContain('observation-changed');

    const denied = new FakeSurfaceSource(), deniedRead = denied.read.bind(denied);
    denied.read = (path, limit) => {
      if (path.endsWith('/intel_pstate/no_turbo')) {
        const error = new Error('denied') as NodeJS.ErrnoException;
        error.code = 'EACCES';
        throw error;
      }
      return deniedRead(path, limit);
    };
    const deniedObservation = collectProgrammeCaptureHostDiagnosticObservationV1(denied);
    expect(deniedObservation.samples[0].fields.turbo)
      .toEqual({ status: 'unavailable', error: 'denied' });
  });

  it('rejects record mutation, positive relabelling, and independent identity substitution', () => {
    const result = rejectionFixture();
    expect(parseProgrammeCaptureHostNonAdmissionV1(
      structuredClone(result.record),
    )).toEqual(result.record);
    for (const mutate of [
      (record: any) => { record.extra = true; },
      (record: any) => { record.outcome = 'eligible'; },
      (record: any) => { record.captureAuthorized = true; },
      (record: any) => { record.reasons = [...record.reasons, record.reasons[0]]; },
      (record: any) => { record.recordDigest = digest('0'); },
      (record: any) => { record.controller.commit = 'a'.repeat(40); },
      (record: any) => { record.beforeStateHead = digest('a'); },
    ]) {
      const record = structuredClone(result.record) as any;
      mutate(record);
      expect(() => parseProgrammeCaptureHostNonAdmissionV1(record)).toThrow();
    }

    const attestation = inputAttestation();
    const state = inputsAttestedState(attestation);
    const anchored = rejectProgrammeCaptureHostPreflightV1({
      state,
      inputAttestation: attestation,
      profileBytes: Buffer.from(PROFILE_TEXT),
      observation: collectProgrammeCaptureHostDiagnosticObservationV1(new FakeSurfaceSource()),
    });
    const rehashed = structuredClone(anchored.record) as any;
    rehashed.runId = 'capture_run_20260828_9999';
    const { recordDigest: _old, ...body } = rehashed;
    rehashed.recordDigest = digestValue(body);
    expect(parseProgrammeCaptureHostNonAdmissionV1(rehashed).runId)
      .toBe('capture_run_20260828_9999');
    expect(() => verifyProgrammeCaptureHostNonAdmissionV1({
      record: rehashed,
      beforeState: state,
      afterState: anchored.state,
      inputAttestation: attestation,
      profileBytes: Buffer.from(PROFILE_TEXT),
    })).toThrow(/AUTHORITY_MISMATCH/);

    const relabelled = structuredClone(anchored.record) as any;
    relabelled.reasons = ['swap-enabled'];
    relabelled.outcome = 'ineligible';
    const { recordDigest: _digest, ...relabelledBody } = relabelled;
    relabelled.recordDigest = digestValue(relabelledBody);
    expect(parseProgrammeCaptureHostNonAdmissionV1(relabelled).outcome).toBe('ineligible');
    expect(() => verifyProgrammeCaptureHostNonAdmissionV1({
      record: relabelled,
      beforeState: state,
      afterState: anchored.state,
      inputAttestation: attestation,
      profileBytes: Buffer.from(PROFILE_TEXT),
    })).toThrow(/SEMANTICS_MISMATCH/);

    const wrongTerminal = transitionProgrammeCaptureStateV1(state, {
      kind: 'fail', evidenceDigest: digest('9'), reasonDigest: digest('8'),
      processDispositionDigest: digest('7'), egressDispositionDigest: digest('6'),
      leaseDispositionDigest: digest('5'),
    });
    expect(() => verifyProgrammeCaptureHostNonAdmissionV1({
      record: anchored.record, beforeState: state, afterState: wrongTerminal,
      inputAttestation: attestation, profileBytes: Buffer.from(PROFILE_TEXT),
    })).toThrow(/TERMINAL_MISMATCH/);

    let coerced = false;
    const poison = structuredClone(anchored.record) as any;
    poison.profileObservation.status = { toString: () => { coerced = true; return 'verified'; } };
    expect(() => parseProgrammeCaptureHostNonAdmissionV1(poison)).toThrow();
    expect(coerced).toBe(false);
  });

  it('binds the input-attested state and cannot be replayed from another phase or claim', () => {
    const attestation = inputAttestation();
    const state = inputsAttestedState(attestation);
    const observation = collectProgrammeCaptureHostDiagnosticObservationV1(new FakeSurfaceSource());
    expect(() => rejectProgrammeCaptureHostPreflightV1({
      state: { ...state, claimDigest: digest('a') } as any,
      inputAttestation: attestation,
      profileBytes: Buffer.from(PROFILE_TEXT),
      observation,
    })).toThrow();
    expect(() => rejectProgrammeCaptureHostPreflightV1({
      state: createProgrammeCaptureStateV1({
        runId: state.runId,
        taskDigest: state.taskDigest,
        claimDigest: state.claimDigest,
        controller: state.controller,
        admissionEvidenceDigest: digest('f'),
      }),
      inputAttestation: attestation,
      profileBytes: Buffer.from(PROFILE_TEXT),
      observation,
    })).toThrow(/INPUTS_ATTESTED_REQUIRED/);
  });

  it('copies hostile byte views without callbacks and classifies the real host without commands', () => {
    let iterated = false;
    class HostileBytes extends Uint8Array {
      *[Symbol.iterator](): Uint8ArrayIterator<number> {
        iterated = true;
        yield* super[Symbol.iterator]();
      }
    }
    const profileBytes = Buffer.from(PROFILE_TEXT);
    const hostile = new HostileBytes(profileBytes.length);
    Uint8Array.prototype.set.call(hostile, profileBytes);
    const hostileResult = rejectionFixture(new FakeSurfaceSource(), hostile);
    expect(hostileResult.record.reasons).toEqual(['positive-control-closure-incomplete']);
    expect(iterated).toBe(false);
    let trapped = false;
    const proxied = new Proxy(hostile, { get: () => { trapped = true; return undefined; } });
    expect(() => rejectionFixture(new FakeSurfaceSource(), proxied)).toThrow(/PROFILE_BYTES_INVALID/);
    expect(trapped).toBe(false);

    const observation = collectProgrammeCaptureHostObservationV1();
    expect(rustArchitecture('x64')).toBe('x86_64');
    expect(rustArchitecture('arm64')).toBe('aarch64');
    expect(rustArchitecture('future-arch')).toBe('unmapped');
    expect(observation.architecture).toBe('x86_64');
    const diagnosis = diagnoseProgrammeCaptureHostObservationV1(observation);
    expect(['ineligible', 'unproven']).toContain(diagnosis.outcome);
    expect(diagnosis.captureAuthorized).toBe(false);
    const attestation = inputAttestation();
    const result = rejectProgrammeCaptureHostPreflightV1({
      state: inputsAttestedState(attestation),
      inputAttestation: attestation,
      profileBytes: undefined,
      observation,
    });
    expect(['ineligible', 'unproven']).toContain(result.record.outcome);
    expect(result.record.captureAuthorized).toBe(false);
    expect(result.record.authority).toBe('controller-classified-non-admission');
    expect(result.state).toMatchObject({ phase: 'failed', captureAttempts: 0 });
  });
});
