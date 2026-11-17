# Windows work-PC setup with MCP

Build **our `crokatron/development` branch**, not the upstream download. SolidCraft and its geometry processing run locally; the AI assistant uses its online service and can receive tool responses/screenshots. This is an experimental development build; see [validation and limitations](../FORK_CHANGELOG.md). Personal scans, designs and settings are not included. Follow your workplace's software and data policies.

## Easiest: ask Codex to set it up

Install/sign in to Codex desktop, open an empty local project folder and paste:

> Set up https://github.com/crokatron/solidcraft on this Windows PC using crokatron/development. Follow docs/work-pc-setup.md and the repository instructions. Check prerequisites, build the app and matching CLI, configure solidcraft-work MCP against the visible app and verify it with read-only inspection. Preserve existing settings/documents, keep one SolidCraft window open, and report failures accurately. Build this branch rather than installing the upstream release.

You may need to complete installer/admin prompts yourself. The manual steps below cover the same setup.

## 1. Install prerequisites once

On 64-bit Windows, install:

1. [Git for Windows](https://git-scm.com/downloads/win).
2. [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/): select **Desktop development with C++**, including the x64/x86 MSVC compiler and Windows SDK. The full IDE is not required.
3. [Rust via rustup](https://rustup.rs/) with the Windows MSVC toolchain. The repository automatically selects its pinned Rust version.
4. Codex desktop, signed in. The separate Codex CLI is optional.

Allow tens of GB of free space for compilation. The first build can take a long time. Reopen your terminal after installation. If the compiler/linker is not found, use **Developer PowerShell for Visual Studio**.

## 2. Clone the correct branch

Run in PowerShell:

```powershell
$sourceRoot = Join-Path $env:USERPROFILE 'src'
New-Item -ItemType Directory -Force -Path $sourceRoot | Out-Null
Set-Location $sourceRoot
git clone --branch crokatron/development https://github.com/crokatron/solidcraft.git solidcraft-work
if ($LASTEXITCODE -ne 0) { throw 'Clone failed; stop and inspect the error.' }
Set-Location (Join-Path $sourceRoot 'solidcraft-work')
git branch --show-current
git rev-parse --short HEAD
```

The branch should be `crokatron/development`. This public clone needs no GitHub login. If the destination already exists, inspect it instead of deleting it. Prefer a short local path outside OneDrive/network folders.

## 3. Build both executables

In that repository folder:

```powershell
$env:CARGO_BUILD_JOBS = '4'
$env:CARGO_PROFILE_RELEASE_LTO = 'false'
$env:CARGO_PROFILE_RELEASE_DEBUG = '0'
cargo build --locked --release -p solidcraft -p solidcraft-cli
if ($LASTEXITCODE -ne 0) { throw 'Build failed; do not use old binaries.' }
& .\target\release\solidcraft-cli.exe --version
```

These settings reduce compilation time/memory; reduce jobs if memory is tight. Outputs are `target\release\solidcraft.exe` and `solidcraft-cli.exe`. The version may still show the upstream base version `0.40.1`; the Git commit identifies our source. A successful build is not a complete test-suite pass.

## 4. Launch one visible window

Save work and close other SolidCraft windows before launching. From the repository folder:

```powershell
$env:SOLIDCRAFT_SETTINGS_DIR = Join-Path $env:LOCALAPPDATA 'SolidCraftCrokatron'
$env:SOLIDCRAFT_UPDATE_CHECK = '0'
& .\target\release\solidcraft.exe --control 27919
```

Keep it open. This isolates preferences and disables upstream update checks. Use this same launch command each day. It may occupy the terminal while the app runs; use a second terminal below. Keep the build folder in place because MCP points to its CLI.

The explicit control port enables local agent access even if the preference is off. No public server or incoming network firewall rule is required. The CLI reads the app's launch token automatically; do not copy tokens between PCs.

## 5. Connect Codex

Choose **one** method. The separate name `solidcraft-work` avoids replacing an existing SolidCraft integration.

### A. If you have the Codex CLI

In a second PowerShell window:

```powershell
$cliPath = Join-Path $env:USERPROFILE 'src\solidcraft-work\target\release\solidcraft-cli.exe'
codex mcp add solidcraft-work -- $cliPath mcp --connect 127.0.0.1:27919
codex mcp list
```

If `codex` is not recognised, use B. If the named entry already exists, inspect/update it instead of duplicating it.

### B. Without the Codex CLI

Back up and open `%USERPROFILE%\.codex\config.toml` (or the configuration in your custom `CODEX_HOME`). Add this table once, preserving all existing settings. Replace `YOUR_WINDOWS_USER` with your actual Windows folder name. If you cloned elsewhere, use that executable's absolute path. TOML does not expand placeholders or `%USERPROFILE%`.

```toml
[mcp_servers.solidcraft-work]
command = 'C:\Users\YOUR_WINDOWS_USER\src\solidcraft-work\target\release\solidcraft-cli.exe'
args = ['mcp', '--connect', '127.0.0.1:27919']
startup_timeout_sec = 30
tool_timeout_sec = 360
```

Single quotes keep Windows backslashes literal. With A, you can add these timeout values to the existing table if needed; do not duplicate it. Restart Codex and start a new local chat after changing configuration. Keep SolidCraft open.

Codex launches this **stdio MCP server**. The address is the CLI-to-app connection, not an HTTP MCP URL. Explicit `--connect` prevents silently working in a separate headless document when the window is closed. See [OpenAI MCP configuration](https://developers.openai.com/codex/mcp/) and [SolidCraft MCP details](mcp.md).

## 6. Verify without changing your design

Ask Codex:

> Use only solidcraft-work MCP. Inspect the current design and take a screenshot of the connected window. Tell me its document/body names. Do not create, edit, save or close anything.

Check that the screenshot/document match your window. A configured server appearing in a list alone does not prove the connection. Then use SolidCraft manually or ask the assistant to model through MCP.

If MCP refuses a file outside its allowed folders, add `--allow-path` and the specific existing project directory to the server arguments. For example, append `'--allow-path', 'C:\CAD\MyProject'` inside the TOML `args` array, using your actual path. Do not grant a whole drive to fix one file error. See [file-access rules](mcp.md#which-files-the-agent-can-reach).

## Updating later

Save work and close SolidCraft. Quit Codex if its MCP process holds the CLI executable open. In the repository directory:

```powershell
git status --short
git pull --ff-only origin crokatron/development
```

If you have local edits or the pull fails, preserve/resolve them before proceeding; do not reset them away. Repeat the build step and launch command, then restart Codex. Pulling source alone does not update the executable.

## Troubleshooting

| Problem | Check |
| --- | --- |
| `cargo` or `git` missing | Complete its installer and reopen the terminal. |
| Missing linker/MSVC/SDK | Install the C++ workload and SDK; use Developer PowerShell. |
| Pinned Rust fails to download | Check network/proxy/toolchain availability; keep the exact error rather than silently changing versions. |
| Missing VC++ runtime DLL | Install Microsoft's supported Visual C++ x64 Redistributable; do not fetch individual DLLs from third parties. |
| No app listening | Start with `--control 27919`; app and MCP ports must match. |
| Port occupied | Identify the existing process; choose another free port in both commands rather than killing an unknown process. |
| No MCP tools | Check absolute path, TOML syntax and enabled state; restart Codex. Workplace policy may restrict MCP. |
| Wrong document | Use `solidcraft-work`, not an older server/plugin; confirm one window and the matching port. |
| Rebuild access denied | Save/close the app and stop its MCP client before rebuilding. |
| Full CI reports Windows error 206 | The current snapshot's CI stopped at `cargo fmt --check` with a command-length error. Full validation is incomplete. |

These steps were checked against the current source/configuration documentation, not on a fresh work PC. Personal files/preferences must be transferred separately. The fork retains upstream licences; public modified binary distribution remains separate from this source setup. See [fork information](../FORK.md).
