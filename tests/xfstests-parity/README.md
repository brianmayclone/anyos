# xfstests Parity Manifest

This directory tracks anyOS parity against the upstream xfstests suite.

Reference snapshot:

- Repository: https://github.com/kdave/xfstests
- Commit: `57d71a884dd1b3b3c44a27d2d106b3be84ddc5fb`
- Commit date: 2026-03-12

Generate or refresh the manifest from a local xfstests checkout:

```sh
python3 tools/xfstests_parity_import.py \
  --xfstests-dir /path/to/xfstests \
  --out tests/xfstests-parity/manifest.csv \
  --summary tests/xfstests-parity/summary.json
```

Validate the checked-in manifest:

```sh
python3 tools/xfstests_parity_check.py tests/xfstests-parity/manifest.csv
```

Manifest status values:

- `native`: implemented as an anyOS-native test.
- `adapted`: implemented with API/tool differences, same bug class.
- `covered`: covered by an existing anyOS test.
- `blocked`: feature, API, or harness support is missing.
- `unsupported`: intentionally not part of CoreFS/exFAT.
- `not-applicable`: belongs to another filesystem/protocol target.
- `todo`: imported but not classified yet.

The long-term goal is that no row remains `todo`.
