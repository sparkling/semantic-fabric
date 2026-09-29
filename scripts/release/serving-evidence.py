#!/usr/bin/env python3
'''Assemble local, unsigned serving evidence: Cargo graph, SPDX 2.3 SBOM, provenance, SHA256SUMS.

A checksum inventory gives integrity of these files only, never authenticity.
'''
import argparse
import datetime
import hashlib
import json
import os
import re
import stat
import sys
import urllib.parse

SCHEMA = 1
PLATFORM = 'linux/amd64'
HOST_TRIPLE = 'x86_64-unknown-linux-gnu'
CRATES_IO = 'registry+https://github.com/rust-lang/crates.io-index'
NOASSERTION = 'NOASSERTION'
MAX_BYTES = 1 << 28
SUMS = 'SHA256SUMS'
ARTIFACT = 'artifact.json'
IMAGE_ID_FILE = 'image.id'
INSPECT = 'image-inspect.json'
ARCHIVE = 'image.tar'
LOCK = 'evidence/Cargo.lock'
METADATA = 'evidence/cargo-metadata.json'
RUSTC = 'evidence/rustc.txt'
TREE = 'evidence/serving-dependencies.txt'
LICENSE_MIT = 'evidence/LICENSE-MIT'
LICENSE_APACHE = 'evidence/LICENSE-APACHE'
GRAPH = 'evidence/cargo-serving-graph.json'
SBOM = 'evidence/sbom.spdx.json'
PROVENANCE = 'evidence/provenance.json'
FROM_IMAGE = (LOCK, METADATA, RUSTC, TREE, LICENSE_MIT, LICENSE_APACHE)
GENERATED = (GRAPH, SBOM, PROVENANCE)
EXPECTED = frozenset((ARTIFACT, IMAGE_ID_FILE, INSPECT, ARCHIVE) + FROM_IMAGE + GENERATED)
OPERATORS = ('AND', 'OR', 'WITH')
IMAGE_RE = re.compile(r'sha256:[0-9a-f]{64}')
REVISION_RE = re.compile(r'[0-9a-f]{40}')
SHA_RE = re.compile(r'[0-9a-f]{64}')
VERSION_RE = re.compile(r'[0-9]+\.[0-9]+\.[0-9]+([.-][a-zA-Z0-9.-]+)?')
CREATED_RE = re.compile(r'[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z')
REL_RE = re.compile(r'[A-Za-z0-9._+-]+(/[A-Za-z0-9._+-]+)*')
TREE_VERSION_RE = re.compile(r'v[0-9]+\.[0-9]+\.[0-9]+\S*')
SBOM_SCOPE = (
    'Cargo crates in the normal+build graph of the sf-cli serving build, read from the exact image. '
    'Base OS packages and the toolchain are not inventoried. Per-edge relationships, download '
    'locations, copyrights and concluded licenses are not asserted. Unsigned; not release-qualified.'
)
SMOKE_NOTE = (
    'Run the existing ignored test minimal_image_serves_all_backends_read_only_and_non_root in '
    'crates/sf-cli/tests/source_tls_live/serving_image.rs with SF_SERVING_IMAGE_ID set to '
    'subject.imageId. Not run here.'
)


class EvidenceError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise EvidenceError(message)


def safe_rel(rel):
    parts = rel.split('/')
    require(REL_RE.fullmatch(rel) and '.' not in parts and '..' not in parts,
            f'unsafe path {rel!r}')


def regular(path, label):
    try:
        info = os.lstat(path)
    except FileNotFoundError:
        raise EvidenceError(f'missing file {label}') from None
    require(not stat.S_ISLNK(info.st_mode), f'symlink not allowed: {label}')
    require(stat.S_ISREG(info.st_mode), f'not a regular file: {label}')
    return info


def read_bytes(bundle, rel):
    path = os.path.join(bundle, rel)
    require(regular(path, rel).st_size <= MAX_BYTES, f'{rel} is too large')
    with open(path, 'rb') as handle:
        return handle.read()


def sha256_file(path, label):
    regular(path, label)
    digest = hashlib.sha256()
    with open(path, 'rb') as handle:
        for block in iter(lambda: handle.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def dumps(document):
    return (json.dumps(document, indent=2, sort_keys=True) + '\n').encode('ascii')


def parse_json(data, label):
    def no_duplicates(pairs):
        keys = [key for key, _ in pairs]
        require(len(keys) == len(set(keys)), f'duplicate key in {label}')
        return dict(pairs)

    try:
        return json.loads(data.decode('utf-8'), object_pairs_hook=no_duplicates)
    except (UnicodeDecodeError, ValueError) as error:
        raise EvidenceError(f'invalid JSON in {label}: {error}') from None


def field(document, key, label, kind=str):
    require(isinstance(document, dict), f'{label} is not an object')
    value = document.get(key)
    require(type(value) is kind, f'{label} field {key} is missing or has the wrong type')
    return value


def check_schema(document, label):
    version = document.get('schemaVersion') if isinstance(document, dict) else None
    require(type(version) is int and version == SCHEMA, f'unsupported schema in {label}')


def check_bundle_dir(bundle):
    trimmed = bundle.rstrip(os.sep) or os.sep
    require(os.path.isdir(trimmed) and not os.path.islink(trimmed),
            'bundle must be a real directory, not a symlink')


def write_file(path, data):
    if os.path.lexists(path):
        regular(path, path)
    with open(path, 'wb') as handle:
        handle.write(data)


def actual_files(bundle):
    '''Regular files to be inventoried. source/ is a build context and is not descended.'''
    found = set()
    for name in sorted(os.listdir(bundle)):
        path = os.path.join(bundle, name)
        info = os.lstat(path)
        require(not stat.S_ISLNK(info.st_mode), f'symlink not allowed: {name}')
        if name == 'source':
            require(stat.S_ISDIR(info.st_mode), 'source must be a directory')
        elif name == 'evidence':
            require(stat.S_ISDIR(info.st_mode), 'evidence must be a directory')
            for inner in sorted(os.listdir(path)):
                rel = 'evidence/' + inner
                regular(os.path.join(path, inner), rel)
                found.add(rel)
        elif name == SUMS:
            regular(path, name)
        else:
            regular(path, name)
            found.add(name)
    return found


def seal(bundle):
    check_bundle_dir(bundle)
    lines = []
    for rel in sorted(actual_files(bundle)):
        safe_rel(rel)
        lines.append(sha256_file(os.path.join(bundle, rel), rel) + '  ' + rel + '\n')
    write_file(os.path.join(bundle, SUMS), ''.join(lines).encode('utf-8'))


def parse_lock(text):
    packages, current, version, in_dependencies = [], None, None, False
    for line in text.splitlines():
        if line == '[[package]]':
            current = {}
            packages.append(current)
            in_dependencies = False
        elif in_dependencies:
            in_dependencies = line != ']'
        elif line.startswith('dependencies = ['):
            in_dependencies = not line.endswith(']')
        elif current is None:
            match = re.fullmatch(r'version = ([0-9]+)', line)
            if match:
                version = int(match.group(1))
        else:
            match = re.fullmatch(r'([a-z]+) = "([^"]*)"', line)
            if match:
                current[match.group(1)] = match.group(2)
    require(version in (3, 4), 'unsupported schema in ' + LOCK)
    require(packages, 'Cargo.lock lists no packages')
    return packages


def parse_tree(text):
    entries = {}
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        head, separator, features = line.partition('|')
        parts = head.split()
        require(separator and len(parts) >= 2 and TREE_VERSION_RE.fullmatch(parts[1]),
                f'serving-dependencies.txt line {number} is not name version|features')
        names = [item.strip() for item in features.replace('(*)', '').split(',')]
        entries.setdefault((parts[0], parts[1][1:]), set()).update(n for n in names if n)
    require(entries, 'serving-dependencies.txt is empty')
    return entries


def parse_rustc(text):
    lines = text.splitlines()
    require(lines and lines[0].startswith('rustc '), 'rustc.txt is not rustc -Vv output')
    info = {}
    for line in lines[1:]:
        key, separator, value = line.partition(': ')
        if separator:
            info[key] = value
    require(info.get('host') == HOST_TRIPLE, 'rustc host is not ' + HOST_TRIPLE)
    require(info.get('release'), 'rustc.txt lacks a release line')
    return {'version': lines[0], 'host': info['host'], 'release': info['release'],
            'commitHash': info.get('commit-hash')}


def spdx_expression_ok(expression):
    depth, want_operand = 0, True
    for token in re.findall(r'\(|\)|[^\s()]+', expression):
        if want_operand:
            if token == '(':
                depth += 1
            elif token not in OPERATORS and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9.+-]*', token):
                want_operand = False
            else:
                return False
        elif token == ')':
            depth -= 1
            if depth < 0:
                return False
        elif token in OPERATORS:
            want_operand = True
        else:
            return False
    return depth == 0 and not want_operand


def load_inputs(bundle, digests=None):
    digests = {} if digests is None else digests

    def digest(rel):
        if rel not in digests:
            digests[rel] = sha256_file(os.path.join(bundle, rel), rel)
        return digests[rel]

    def text(rel):
        try:
            return read_bytes(bundle, rel).decode('utf-8')
        except UnicodeDecodeError:
            raise EvidenceError(f'{rel} is not UTF-8') from None

    artifact = parse_json(read_bytes(bundle, ARTIFACT), ARTIFACT)
    check_schema(artifact, ARTIFACT)
    revision = field(artifact, 'sourceRevision', ARTIFACT)
    crate = field(artifact, 'crateVersion', ARTIFACT)
    package = field(artifact, 'packageVersion', ARTIFACT)
    image_id = field(artifact, 'imageId', ARTIFACT)
    archive = field(artifact, 'imageArchive', ARTIFACT, dict)
    require(REVISION_RE.fullmatch(revision), 'artifact sourceRevision is not a full commit ID')
    require(VERSION_RE.fullmatch(crate), 'artifact crateVersion is malformed')
    require(package == crate + '-git.' + revision[:12],
            'artifact packageVersion does not match crateVersion and sourceRevision')
    require(IMAGE_RE.fullmatch(image_id), 'artifact imageId is not a sha256 image ID')
    require(field(artifact, 'platform', ARTIFACT) == PLATFORM, 'artifact platform is not ' + PLATFORM)
    require(field(artifact, 'admission', ARTIFACT).startswith('unqualified'),
            'artifact admission must stay unqualified')
    field(artifact, 'buildCommand', ARTIFACT)
    require(archive.get('path') == ARCHIVE, 'artifact imageArchive path is not image.tar')
    require(archive.get('sha256') == digest(ARCHIVE),
            'artifact imageArchive sha256 does not match image.tar')
    require(field(artifact, 'cargoLockSha256', ARTIFACT) == digest(LOCK),
            'artifact cargoLockSha256 does not match ' + LOCK)
    recorded = read_bytes(bundle, IMAGE_ID_FILE).decode('ascii', 'replace')
    require(recorded.removesuffix('\n') == image_id, 'image.id does not match artifact.json imageId')

    inspect = parse_json(read_bytes(bundle, INSPECT), INSPECT)
    require(isinstance(inspect, list) and len(inspect) == 1 and isinstance(inspect[0], dict),
            'image-inspect.json must describe exactly one image')
    entry = inspect[0]
    require(entry.get('Id') == image_id, 'image-inspect.json Id does not match artifact.json imageId')
    require(entry.get('Os') == 'linux' and entry.get('Architecture') == 'amd64',
            'inspected image is not linux/amd64')
    config = entry.get('Config')
    labels = config.get('Labels') if isinstance(config, dict) else None
    require(isinstance(labels, dict), 'image-inspect.json lacks Config.Labels')
    require(labels.get('org.opencontainers.image.revision') == revision,
            'image label revision does not match artifact sourceRevision')
    require(labels.get('org.opencontainers.image.version') == package,
            'image label version does not match artifact packageVersion')

    metadata = parse_json(read_bytes(bundle, METADATA), METADATA)
    require(isinstance(metadata, dict) and type(metadata.get('version')) is int
            and metadata['version'] == 1, 'unsupported schema in ' + METADATA)
    tree = parse_tree(text(TREE))
    require(('sf-cli', crate) in tree, 'sf-cli at the artifact crateVersion is not in the serving graph')
    digest(LICENSE_MIT)
    digest(LICENSE_APACHE)
    return {'bundle': bundle, 'artifact': artifact, 'image_id': image_id, 'revision': revision,
            'crate': crate, 'package': package, 'tree': tree, 'lock': parse_lock(text(LOCK)),
            'metadata': metadata, 'toolchain': parse_rustc(text(RUSTC)), 'digest': digest}


def make_graph(ctx):
    locked, described = {}, {}
    for item in ctx['lock']:
        locked.setdefault((item.get('name'), item.get('version')), []).append(item)
    packages = ctx['metadata'].get('packages')
    require(isinstance(packages, list) and all(isinstance(p, dict) for p in packages),
            'cargo metadata lacks a packages list')
    for item in packages:
        described.setdefault((item.get('name'), item.get('version')), []).append(item)
    records = []
    for (name, version), features in sorted(ctx['tree'].items()):
        lock_hits = locked.get((name, version), [])
        meta_hits = described.get((name, version), [])
        require(len(lock_hits) == 1, f'{name} {version} is not exactly once in Cargo.lock')
        require(len(meta_hits) == 1, f'{name} {version} is not exactly once in cargo metadata')
        source = lock_hits[0].get('source')
        require(source == meta_hits[0].get('source'),
                f'{name} {version} source differs between Cargo.lock and cargo metadata')
        license_expression = meta_hits[0].get('license')
        require(license_expression is None or isinstance(license_expression, str),
                f'{name} {version} license in cargo metadata is not a string')
        checksum = lock_hits[0].get('checksum')
        require(checksum is None or SHA_RE.fullmatch(checksum), f'{name} {version} checksum is malformed')
        records.append({'name': name, 'version': version, 'source': source, 'checksum': checksum,
                        'license': license_expression, 'features': sorted(features)})
    return {
        'schemaVersion': SCHEMA,
        'kind': 'semantic-fabric-serving-cargo-graph',
        'selection': 'cargo tree --locked -p sf-cli --no-default-features --edges normal,build (host linux/amd64)',
        'featureSource': TREE,
        'metadataSource': METADATA + ' (license, source)',
        'checksumSource': LOCK + ' (registry .crate SHA-256; null when the lock has none)',
        'root': {'name': 'sf-cli', 'version': ctx['crate']},
        'packages': records,
    }


def sbom_package(number, record):
    source = record['source']
    declared = record['license'] if spdx_expression_ok(record['license'] or '') else NOASSERTION
    note = 'cargo source: ' + (source or 'workspace path')
    note += '; enabled features: ' + (','.join(record['features']) or 'none')
    if record['license'] and declared == NOASSERTION:
        note += '; cargo license field is not a valid SPDX expression: ' + record['license']
    elif not record['license']:
        note += '; cargo metadata declares no license expression'
    package = {
        'SPDXID': f'SPDXRef-Package-{number}',
        'name': record['name'],
        'versionInfo': record['version'],
        'downloadLocation': NOASSERTION,
        'filesAnalyzed': False,
        'licenseConcluded': NOASSERTION,
        'licenseDeclared': declared,
        'copyrightText': NOASSERTION,
        'comment': note,
    }
    if record['checksum']:
        package['checksums'] = [{'algorithm': 'SHA256', 'checksumValue': record['checksum']}]
    if source == CRATES_IO:
        quoted = urllib.parse.quote(record['name'], safe='') + '@' + urllib.parse.quote(record['version'], safe='')
        package['externalRefs'] = [{'referenceCategory': 'PACKAGE-MANAGER', 'referenceType': 'purl',
                                    'referenceLocator': 'pkg:cargo/' + quoted}]
    return package


def make_sbom(ctx, graph, created):
    root = [r for r in graph['packages'] if r['name'] == 'sf-cli' and r['version'] == ctx['crate']][0]
    packages = [sbom_package(number, record) for number, record in enumerate(graph['packages'], 1)]
    image_declared = root['license'] if spdx_expression_ok(root['license'] or '') else NOASSERTION
    image = {
        'SPDXID': 'SPDXRef-Image',
        'name': 'semantic-fabric',
        'versionInfo': ctx['package'],
        'downloadLocation': NOASSERTION,
        'filesAnalyzed': False,
        'licenseConcluded': NOASSERTION,
        'licenseDeclared': image_declared,
        'copyrightText': NOASSERTION,
        'checksums': [{'algorithm': 'SHA256', 'checksumValue': ctx['artifact']['imageArchive']['sha256']}],
        'comment': 'local linux/amd64 image; imageId ' + ctx['image_id'] + '; source revision '
                   + ctx['revision'] + '; checksum is of image.tar',
    }
    return {
        'spdxVersion': 'SPDX-2.3',
        'dataLicense': 'CC0-1.0',
        'SPDXID': 'SPDXRef-DOCUMENT',
        'name': 'semantic-fabric-serving-' + ctx['package'],
        'documentNamespace': 'urn:semantic-fabric:sbom:' + ctx['revision'] + ':'
                             + ctx['image_id'].split(':', 1)[1],
        'creationInfo': {'created': created, 'creators': ['Tool: semantic-fabric-serving-evidence-1']},
        'comment': SBOM_SCOPE,
        'documentDescribes': ['SPDXRef-Image'],
        'packages': [image] + packages,
        'relationships': [{'spdxElementId': 'SPDXRef-Image', 'relationshipType': 'DEPENDS_ON',
                           'relatedSpdxElement': p['SPDXID']} for p in packages],
    }


def make_provenance(ctx, graph_bytes, sbom_bytes):
    files = {'artifactManifest': ARTIFACT, 'cargoLock': LOCK, 'cargoMetadata': METADATA,
             'imageInspect': INSPECT, 'licenseApache': LICENSE_APACHE, 'licenseMit': LICENSE_MIT,
             'rustc': RUSTC, 'servingDependencies': TREE}
    materials = {key: {'path': rel, 'sha256': ctx['digest'](rel)} for key, rel in files.items()}
    materials['servingGraph'] = {'path': GRAPH, 'sha256': sha256_bytes(graph_bytes)}
    materials['sbom'] = {'path': SBOM, 'sha256': sha256_bytes(sbom_bytes)}
    artifact = ctx['artifact']
    return {
        'schemaVersion': SCHEMA,
        'kind': 'semantic-fabric-serving-provenance',
        'signature': {'status': 'unsigned',
                      'note': 'no signing identity exists; checksums give integrity, not authenticity'},
        'sourceRevision': ctx['revision'],
        'crateVersion': ctx['crate'],
        'packageVersion': ctx['package'],
        'platform': PLATFORM,
        'subject': {'imageId': ctx['image_id'],
                    'imageArchive': {'path': ARCHIVE, 'sha256': artifact['imageArchive']['sha256']}},
        'build': {'command': artifact['buildCommand'], 'containerfile': 'scripts/release/Containerfile'},
        'toolchain': ctx['toolchain'],
        'materials': materials,
        'admission': 'unqualified',
        'receiptBinding': 'A later smoke, scan, waiver or signature receipt applies only if it cites '
                          'subject.imageId and sourceRevision; a movable tag or another image ID does not qualify.',
        'unmet': [
            {'requirement': 'release signature', 'status': 'unmet',
             'note': 'nothing was signed and no signing identity was invented'},
            {'requirement': 'advisory review or waivers', 'status': 'unmet',
             'note': 'no advisory scan was run and no waiver was invented'},
            {'requirement': 'live exact-image smoke receipt', 'status': 'unmet', 'note': SMOKE_NOTE},
        ],
    }


def build_documents(ctx, created):
    graph_bytes = dumps(make_graph(ctx))
    graph = json.loads(graph_bytes.decode('ascii'))
    sbom_bytes = dumps(make_sbom(ctx, graph, created))
    provenance_bytes = dumps(make_provenance(ctx, graph_bytes, sbom_bytes))
    return {GRAPH: graph_bytes, SBOM: sbom_bytes, PROVENANCE: provenance_bytes}


def assemble(bundle, epoch):
    check_bundle_dir(bundle)
    evidence = os.path.join(bundle, 'evidence')
    require(os.path.isdir(evidence) and not os.path.islink(evidence), 'evidence directory is missing')
    allowed = {rel.split('/', 1)[1] for rel in FROM_IMAGE + GENERATED}
    unexpected = sorted(set(os.listdir(evidence)) - allowed)
    require(not unexpected, f'unexpected exported file(s): {unexpected}')
    try:
        moment = datetime.datetime.fromtimestamp(epoch, tz=datetime.timezone.utc)
    except (OverflowError, OSError, ValueError):
        raise EvidenceError('source date epoch is out of range') from None
    created = moment.strftime('%Y-%m-%dT%H:%M:%SZ')
    for rel, data in build_documents(load_inputs(bundle), created).items():
        write_file(os.path.join(bundle, rel), data)
    seal(bundle)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    build = commands.add_parser('assemble', help='derive graph, SBOM, provenance and SHA256SUMS')
    build.add_argument('bundle')
    build.add_argument('--source-date-epoch', type=int, required=True)
    commands.add_parser('seal', help='rewrite SHA256SUMS from the files present').add_argument('bundle')
    args = parser.parse_args(argv)
    try:
        if args.command == 'assemble':
            assemble(args.bundle, args.source_date_epoch)
        else:
            seal(args.bundle)
    except (EvidenceError, OSError) as error:
        print(f'serving-evidence: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
