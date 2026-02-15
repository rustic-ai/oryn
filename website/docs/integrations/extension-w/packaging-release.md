# Oryn-W Packaging and Preview Release

## Local Packaging

Build and package from repo root:

```bash
./scripts/build-extension-w.sh
./scripts/pack-extension-w.sh
```

Artifacts are written to `dist/`:

- `oryn-w-<version>.zip`
- `oryn-w-<version>.sha256`
- `oryn-w-<version>.txt`

## Preview Release Workflow

GitHub preview release automation:

- workflow: `.github/workflows/preview-release.yml`
- trigger: push tag matching `preview-v*`
- output: packaged extension artifacts published as a prerelease

Example tag:

```bash
git tag preview-v0.2.0-rc1
git push origin preview-v0.2.0-rc1
```

## Validation in Preview Workflow

Preview workflow runs:

- Rust checks (`fmt`, `clippy`, tests)
- JS checks (scanner + extension)
- quick E2E suite
- extension build + package
- release artifact upload

## Integrity Check

Verify local package checksum:

```bash
sha256sum dist/oryn-w-*.zip
cat dist/oryn-w-*.sha256
```

## GitHub Pages Docs

Docs deployment is separate from extension packaging:

- workflow: `.github/workflows/docs-pages.yml`
- builds MkDocs from `website/`
- deploys to GitHub Pages
