#!/usr/bin/env python3
'''Offline verifier for a local serving bundle.

Checks integrity and internal consistency only. It never establishes authenticity:
the bundle is unsigned and admission stays unqualified.
'''
import argparse
import importlib.util
import os
import re
import sys
import tarfile

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    'serving_evidence',
    os.path.join(os.path.dirname(os.path.abspath(__file__)), 'serving-evidence.py'))
E = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(E)
SUM_LINE = re.compile(r'([0-9a-f]{64})  (.+)')
DIGEST_RE = re.compile(r'sha256:([0-9a-f]{64})')
MAX_MEMBERS = 100000
MAX_JSON = 1 << 24
BACKSLASH = chr(92)
NUL = chr(0)
PLAIN_TYPES = (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE)
LINK_KINDS = {tarfile.SYMTYPE: 'symlink', tarfile.LNKTYPE: 'hardlink'}
MANIFEST_TYPES = (
    'application/vnd.oci.image.manifest.v1+json',
    'application/vnd.docker.distribution.manifest.v2+json',
)
CONFIG_TYPES = (
    'application/vnd.oci.image.config.v1+json',
    'application/vnd.docker.container.image.v1+json',
)


def parse_sums(data):
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError:
        raise E.EvidenceError('malformed SHA256SUMS: not UTF-8') from None
    E.require(text.endswith('\n') and text.strip(), 'malformed SHA256SUMS: empty or unterminated')
    declared = {}
    for line in text[:-1].split('\n'):
        match = SUM_LINE.fullmatch(line)
        E.require(match, 'malformed SHA256SUMS line')
        rel = match.group(2)
        E.safe_rel(rel)
        E.require(rel not in declared, f'duplicate SHA256SUMS path {rel!r}')
        declared[rel] = match.group(1)
    return declared


def member_name(member):
    '''Normalized safe name of a tar member, or None for the archive root directory.'''
    name = member.name[2:] if member.name.startswith('./') else member.name
    if member.isdir() and name in ('', '.'):
        return None
    E.require(name and BACKSLASH not in name and NUL not in name
              and all(part not in ('', '.', '..') for part in name.split('/')),
              f'unsafe image.tar member path {member.name!r}')
    kind = LINK_KINDS.get(member.type, 'special file')
    E.require(member.type in PLAIN_TYPES,
              f'image.tar member {name!r} is a {kind}; only regular files and directories are allowed')
    return name


class Image:
    '''Header index of image.tar. Member data is read in memory only; nothing is extracted.'''

    def __init__(self, archive):
        self.archive = archive
        self.members = {}
        for count, member in enumerate(archive, 1):
            E.require(count <= MAX_MEMBERS, 'image.tar has too many members')
            name = member_name(member)
            if name is not None:
                E.require(name not in self.members, f'duplicate image.tar member {name!r}')
                self.members[name] = member

    def regular(self, name, label):
        member = self.members.get(name)
        E.require(member is not None, f'image.tar lacks {label} {name!r}')
        E.require(member.isreg(), f'image.tar {label} {name!r} is not a regular file')
        return member

    def read(self, name, label):
        member = self.regular(name, label)
        E.require(member.size <= MAX_JSON, f'image.tar {label} {name!r} is too large')
        data = self.archive.extractfile(member).read()
        E.require(len(data) == member.size, f'image.tar {label} {name!r} is truncated')
        return data


def blob_path(hex_digest):
    return 'blobs/sha256/' + hex_digest


def descriptor(value, media_types, label):
    E.require(isinstance(value, dict), f'{label} descriptor is not an object')
    media = value.get('mediaType')
    E.require(isinstance(media, str) and (media_types is None or media in media_types),
              f'{label} descriptor has an unsupported mediaType')
    size = value.get('size')
    E.require(type(size) is int and size >= 0, f'{label} descriptor size is invalid')
    digest = value.get('digest')
    match = DIGEST_RE.fullmatch(digest) if isinstance(digest, str) else None
    E.require(match, f'{label} descriptor digest is not sha256:<64 hex>')
    return match.group(1), size


def read_blob(image, hex_digest, size, label):
    data = image.read(blob_path(hex_digest), label)
    E.require(len(data) == size, f'{label} size differs from its descriptor')
    E.require(E.sha256_bytes(data) == hex_digest, f'{label} digest does not match its descriptor')
    return data


def check_layer(image, value):
    hex_digest, size = descriptor(value, None, 'layer')
    member = image.regular(blob_path(hex_digest), 'layer blob')
    E.require(member.size == size, 'layer blob size differs from its descriptor')


def check_docker(image, image_hex):
    manifest = E.parse_json(image.read('manifest.json', 'Docker manifest'), 'manifest.json')
    E.require(isinstance(manifest, list) and len(manifest) == 1 and isinstance(manifest[0], dict),
              'manifest.json must describe exactly one image')
    config_path = manifest[0].get('Config')
    layers = manifest[0].get('Layers')
    E.require(isinstance(config_path, str), 'manifest.json entry lacks a Config path')
    E.safe_rel(config_path)
    E.require(isinstance(layers, list) and layers and all(isinstance(item, str) for item in layers),
              'manifest.json entry lacks a Layers list')
    for item in layers:
        E.safe_rel(item)
        image.regular(item, 'Docker layer')
    data = image.read(config_path, 'Docker config')
    E.require(E.sha256_bytes(data) == image_hex, 'Docker config digest does not match imageId')
    return data


def check_manifest(image, document, image_hex):
    E.require(isinstance(document, dict) and type(document.get('schemaVersion')) is int
              and document['schemaVersion'] == 2, 'unsupported schema in OCI manifest')
    config_hex, config_size = descriptor(document.get('config'), CONFIG_TYPES, 'manifest config')
    if config_hex != image_hex:
        return False
    read_blob(image, config_hex, config_size, 'manifest config')
    layers = document.get('layers')
    E.require(isinstance(layers, list) and layers, 'OCI manifest lists no layers')
    for value in layers:
        check_layer(image, value)
    return True


def check_oci(image, image_hex):
    layout = E.parse_json(image.read('oci-layout', 'oci-layout'), 'oci-layout')
    E.require(isinstance(layout, dict) and layout.get('imageLayoutVersion') == '1.0.0',
              'unsupported schema in oci-layout')
    index = E.parse_json(image.read('index.json', 'OCI index'), 'index.json')
    E.require(isinstance(index, dict) and type(index.get('schemaVersion')) is int
              and index['schemaVersion'] == 2, 'unsupported schema in index.json')
    entries = index.get('manifests')
    E.require(isinstance(entries, list) and entries, 'index.json lists no manifests')
    linked = set()
    for entry in entries:
        manifest_hex, size = descriptor(entry, MANIFEST_TYPES, 'index entry')
        document = E.parse_json(read_blob(image, manifest_hex, size, 'index entry'), 'OCI manifest')
        if check_manifest(image, document, image_hex):
            linked.add(manifest_hex)
    E.require(linked, 'no OCI manifest links to the config named by imageId')
    E.require(len(linked) == 1, 'more than one OCI manifest links to the config named by imageId')
    return image.read(blob_path(image_hex), 'manifest config')


def check_config(data, ctx):
    config = E.parse_json(data, 'image config')
    E.require(isinstance(config, dict) and config.get('os') == 'linux'
              and config.get('architecture') == 'amd64', 'image config is not linux/amd64')
    rootfs = config.get('rootfs')
    diffs = rootfs.get('diff_ids') if isinstance(rootfs, dict) else None
    E.require(isinstance(rootfs, dict) and rootfs.get('type') == 'layers'
              and isinstance(diffs, list) and diffs
              and all(isinstance(item, str) and DIGEST_RE.fullmatch(item) for item in diffs),
              'image config lacks rootfs layer digests')
    inner = config.get('config')
    labels = inner.get('Labels') if isinstance(inner, dict) else None
    E.require(isinstance(labels, dict)
              and labels.get('org.opencontainers.image.revision') == ctx['revision']
              and labels.get('org.opencontainers.image.version') == ctx['package'],
              'image config labels do not match artifact sourceRevision and packageVersion')


def check_archive(path, ctx):
    image_hex = ctx['image_id'].split(':', 1)[1]
    try:
        with tarfile.open(path, 'r:') as archive:
            image = Image(archive)
            docker = 'manifest.json' in image.members
            oci = 'index.json' in image.members or 'oci-layout' in image.members
            E.require(docker or oci, 'image.tar has neither manifest.json nor an OCI index.json')
            configs = []
            if docker:
                configs.append(check_docker(image, image_hex))
            if oci:
                configs.append(check_oci(image, image_hex))
            check_config(configs[0], ctx)
    except (tarfile.TarError, OSError, EOFError) as error:
        raise E.EvidenceError(f'image.tar is not a readable tar: {error}') from None


def verify(bundle, expect_image, expect_revision):
    E.check_bundle_dir(bundle)
    declared = parse_sums(E.read_bytes(bundle, E.SUMS))
    actual = E.actual_files(bundle)
    missing = sorted(set(declared) - actual)
    E.require(not missing, f'missing file(s) declared in SHA256SUMS: {missing}')
    undeclared = sorted(actual - set(declared))
    E.require(not undeclared, f'undeclared file(s) not in SHA256SUMS: {undeclared}')
    lacking = sorted(E.EXPECTED - set(declared))
    E.require(not lacking, f'SHA256SUMS lacks required file(s): {lacking}')
    unexpected = sorted(set(declared) - E.EXPECTED)
    E.require(not unexpected, f'unexpected declared file(s): {unexpected}')
    digests = {}
    for rel, expected in declared.items():
        digests[rel] = E.sha256_file(os.path.join(bundle, rel), rel)
        E.require(digests[rel] == expected, f'digest mismatch for {rel}')
    ctx = E.load_inputs(bundle, digests)
    E.require(expect_image is None or expect_image == ctx['image_id'],
              'bundle imageId differs from expected image ID')
    E.require(expect_revision is None or expect_revision == ctx['revision'],
              'bundle sourceRevision differs from expected source revision')
    check_archive(os.path.join(bundle, E.ARCHIVE), ctx)
    source_lock = os.path.join(bundle, 'source', 'Cargo.lock')
    if os.path.lexists(source_lock):
        E.require(E.sha256_file(source_lock, 'source/Cargo.lock') == ctx['artifact']['cargoLockSha256'],
                  'source/Cargo.lock differs from artifact cargoLockSha256')
    for rel in (E.GRAPH, E.PROVENANCE):
        E.check_schema(E.parse_json(E.read_bytes(bundle, rel), rel), rel)
    sbom = E.parse_json(E.read_bytes(bundle, E.SBOM), E.SBOM)
    E.require(isinstance(sbom, dict) and sbom.get('spdxVersion') == 'SPDX-2.3',
              'unsupported schema in ' + E.SBOM)
    created = E.field(E.field(sbom, 'creationInfo', E.SBOM, dict), 'created', E.SBOM)
    E.require(E.CREATED_RE.fullmatch(created), 'SBOM creation time is malformed')
    for rel, expected in E.build_documents(ctx, created).items():
        E.require(E.read_bytes(bundle, rel) == expected,
                  f'{rel} does not match recomputation from the exported evidence')
    return ctx, len(declared)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('bundle')
    parser.add_argument('--expect-image-id')
    parser.add_argument('--expect-revision')
    args = parser.parse_args(argv)
    try:
        ctx, count = verify(args.bundle, args.expect_image_id, args.expect_revision)
    except E.EvidenceError as error:
        print(f'verify-serving-bundle: {error}', file=sys.stderr)
        return 1
    except Exception as error:
        print(f'verify-serving-bundle: internal error, bundle treated as invalid: {error!r}',
              file=sys.stderr)
        return 1
    print(f"OK: {count} files match SHA256SUMS and the recomputed evidence for "
          f"{ctx['image_id']} at {ctx['revision']}.")
    print('Integrity only: no signature or authenticity is established. Admission stays unqualified '
          '(live smoke, advisory review or waivers and signing are unmet).')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
