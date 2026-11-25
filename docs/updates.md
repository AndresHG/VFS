# App updates: how they work, the signing keys, releasing, testing

Spec: [VFS-21](../specs/vfs-21-updates/spec.md). Code: `apps/desktop/src-tauri/src/updates.rs`,
`apps/desktop/ui/updates.js`, `.github/workflows/release.yml`, `scripts/release_manifest.py`.

## How an update reaches a user

1. Ten seconds after the window opens, the UI asks the backend `check_for_update`. With
   "Buscar actualizaciones al abrir" off (`updates.json` in the local app-data folder), the
   backend answers "no update" without any connection.
2. Otherwise the Tauri updater plugin downloads
   `https://github.com/AndresHG/VFS/releases/latest/download/latest.json` (20 s timeout).
   GitHub serves it from the newest **published, non-prerelease** release. A failure is
   logged to `logs/vfs.log` and no notice is shown (only Ajustes › Acerca de says the check
   failed, VFS-45); the next launch tries again.
3. If the manifest's version is newer (`updates::is_newer`; a stable build is never offered a
   pre-release), a notice offers "Actualizar" / "Ahora no".
4. "Actualizar" downloads the platform's asset, checks its minisign signature against the
   public key in `tauri.conf.json` (`plugins.updater.pubkey`), and only then installs. On
   both platforms the content run is stopped first: the document being read gets 5 s, then
   the `vfs-extract` worker is killed, so the installer can replace it (VFS-27 R3). Then:
   - **Windows**: the NSIS `*-setup.exe` runs in passive mode (progress window, no
     questions, per-user so no UAC). The app stops the indexer, exits, and the installer
     reopens it.
   - **macOS**: `VFS.app.tar.gz` replaces the running `VFS.app` in place; the app stops the
     indexer and relaunches.
   A failed download or a bad signature installs nothing; the notice offers "Reintentar".
   If the Windows installer cannot start after the indexer was stopped, the app relaunches
   on the current version instead.

## The signing key

- The **private key** (minisign, password-protected) and its **password** are the GitHub
  Actions secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Only
  `release.yml` reads them. GitHub secrets cannot be read back, so they are not a backup.
- The maintainer keeps the only other copy outside the repository (key file, `.pub` file and
  password file in the maintainer's `~/.tauri/` folder) and must back them up somewhere
  durable and private, such as a password manager entry with the three files attached.
- The **public key** is committed in `tauri.conf.json`; it is not secret.
- **If the private key or its password is lost**, no new release can be signed with it, and
  every installed copy keeps rejecting updates signed with any other key. Users would have to
  download and install a new version by hand once (a release with a new public key), after
  which updates work again. Never commit the private key or paste it anywhere.
- To rotate (key leaked): generate a new pair (`npx @tauri-apps/cli signer generate -p
  <password> -w <file>`), set both secrets, put the new public key in `tauri.conf.json`, and
  release. Copies still on the old key update one last time only if that release is also
  signed with the old key; otherwise they need a manual install.

## The macOS signing identity

Spec: [VFS-28](../specs/vfs-28-ask-archive/spec.md) R3, AC-3, D6, D12. Script:
`scripts/macos_signing.py`. This is OS code signing, separate from the updater key above.

- Every macOS release is signed with one self-signed certificate, «VFS Self-Signed Code
  Signing», valid for 20 years. No Apple account is involved, and the app is still not
  notarized. With one stable identity, a keychain item (an AI key) saved by one version stays
  readable by the next without a prompt. An ad-hoc signature changes with every build, so
  the next version would be denied the item.
- The signature has no Team ID. Its designated requirement is the app identifier plus the
  certificate's hash, the same for every build, and that is what the keychain checks.
- The certificate and its key are the Actions secrets `MACOS_SIGNING_CERTIFICATE` (a base64
  .p12) and `MACOS_SIGNING_CERTIFICATE_PASSWORD`. Only `release.yml` reads them. The
  maintainer keeps the offline backup: the three files `new-cert` writes, in the same private
  place as the updater key.
- To make it (once, on the maintainer's Mac): run `python3 scripts/macos_signing.py new-cert
  --out <private folder>`, then the two `gh secret set` commands it prints. Never commit the
  files or paste them anywhere.
- **If the certificate is lost or replaced**, the next release cannot read the keys that
  users saved. The app says «No se puede leer la clave guardada; vuelva a introducirla» and
  keeps working without AI. Each user types the key once more, and macOS may ask once more
  for folder access. Nothing crashes, and search is not affected.
- The first release signed this way (v0.4) changes the signature once. Its release notes say
  that macOS may ask once again for folder access (D12).
- A v0.3.x hotfix is cut from the `v0.3.0` tag, not from `main`, so it keeps the ad-hoc
  signature. The signature change ships only in v0.4.
- Checks: every pull request runs AC-3 on the macOS runner with a throwaway certificate
  (`ci.yml`). Each release runs it again with the real one, after `check-app` confirms that
  the app and the `vfs-extract` worker carry the identity (`release.yml`).

## Releasing

1. Bump `version` in `apps/desktop/src-tauri/tauri.conf.json` and the workspace
   `Cargo.toml`, merge to `main`.
2. `git tag v<version> && git push origin v<version>`. `release.yml` builds and signs both
   platforms, writes `latest.json`, and creates a **draft** release with the installer, the
   macOS zip, `VFS.app.tar.gz`, the `.sig` files, `latest.json` and `SHA256SUMS.txt`.
3. Check the draft, then publish it. From that moment installed apps offer the update.
   Marking it "pre-release" keeps it away from users (the `latest` endpoint skips it).

A `workflow_dispatch` run (`gh workflow run release.yml --ref <branch>`) is a dry run: the
same signed artifacts and `latest.json` in the `vfs-release` artifact, no release.

## Testing an update by hand

The real endpoint only serves published releases, so a local test points a build at a local
manifest instead, with a throwaway key and a separate app identifier (its own data folder):

```sh
npx @tauri-apps/cli@2.12.6 signer generate --ci -p test -w /tmp/vfs-test.key
# conf-<v>.json: {"version":"<v>","identifier":"app.vfs.desktop.updatetest",
#   "bundle":{"createUpdaterArtifacts":true},
#   "plugins":{"updater":{"pubkey":"<content of /tmp/vfs-test.key.pub>",
#     "endpoints":["http://127.0.0.1:8765/latest.json"],
#     "dangerousInsecureTransportProtocol":true}}}
export TAURI_SIGNING_PRIVATE_KEY="$(cat /tmp/vfs-test.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=test
# in apps/desktop/src-tauri:
npx @tauri-apps/cli@2.12.6 build --bundles app --config conf-0.2.1.json   # Windows: --bundles nsis
# copy the updater artifact and its .sig into serve/, then build 0.2.0 the same way
python3 scripts/release_manifest.py --version 0.2.1 --base-url http://127.0.0.1:8765 \
  --platform darwin-aarch64=serve/VFS.app.tar.gz --out serve/latest.json
# (Windows: --platform windows-x86_64=serve/VFS_0.2.1_x64-setup.exe)
python3 -m http.server 8765 --directory serve
```

Open the 0.2.0 build, wait ten seconds, click "Actualizar": the app must reopen as 0.2.1.
Repeat with a corrupted `.sig` (or the server stopped mid-download) to see "Reintentar" and
the old version still running. On Windows, install the 0.2.0 setup.exe first, then do the
same; the installer's progress window appears and VFS reopens as 0.2.1.
