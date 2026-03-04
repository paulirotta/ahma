# Winget Packaging

This directory contains the [Windows Package Manager](https://learn.microsoft.com/en-us/windows/package-manager/)
manifest templates for Ahma.

## Package ID

`paulirotta.Ahma`

## Install (once published)

```powershell
winget install paulirotta.Ahma
```

This installs both `ahma_mcp.exe` and `ahma_simplify.exe` as portable commands
accessible from any terminal.

## Manifest structure

| File | Purpose |
|------|---------|
| `paulirotta.Ahma.yaml` | Version manifest |
| `paulirotta.Ahma.locale.en-US.yaml` | English description / metadata |
| `paulirotta.Ahma.installer.yaml` | Installer definition (zip + portable) |

The templates in this directory use `PackageVersion: 0.5.0` and
`InstallerSha256: <SHA256_PLACEHOLDER>`.  The `scripts/publish-winget.sh`
script instantiates them with the real version and checksum for each release.

## Publishing a new version

### Automated (CI)

The `job-publish-winget` CI job runs automatically after a successful release
on `main`.  It requires a repository secret `WINGET_TOKEN` with write access
to [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).

### Manual

```bash
# Install wingetcreate (Windows only)
winget install Microsoft.WingetCreate

# From the repo root:
export WINGET_TOKEN=<your-github-token>
./scripts/publish-winget.sh 0.5.0
```

## Requirements

- **Runtime**: PowerShell 7+ (`pwsh`) must be installed for `ahma_mcp` to function.
  ```powershell
  winget install Microsoft.PowerShell
  ```
- **Windows**: 10 or later (x64).

## Submission checklist

Before submitting to `winget-pkgs`:

- [ ] Windows release `.zip` artifact is published on GitHub
- [ ] SHA256SUMS file is present in the release
- [ ] `wingetcreate validate` passes locally
- [ ] `ahma_mcp --version` works after a clean `winget install`
- [ ] `ahma_mcp --no-sandbox` confirms the binary runs (sandbox backend is pending)
