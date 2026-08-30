#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

fail() {
  echo "PostgreSQL key-store dependency guard: $*" >&2
  exit 1
}

grep -Fq 'security-framework = "=3.7.0"' Cargo.toml || fail "security-framework pin drift"
grep -Fq 'security-framework-sys = "=2.17.0"' Cargo.toml || fail "security-framework-sys pin drift"
grep -Fq '"Win32_Security_Credentials"' Cargo.toml || fail "Credential Manager windows-sys feature missing"
grep -Fq 'openbot-infra = { path = "crates/openbot-infra", default-features = false }' Cargo.toml || fail "workspace infra defaults must stay disabled"
grep -Fq 'default = ["server-sso"]' crates/openbot-infra/Cargo.toml || fail "infra Server SSO default drift"
grep -Fq 'desktop-local = []' crates/openbot-infra/Cargo.toml || fail "infra Desktop Local feature missing"
grep -Fq 'server-runtime = [' crates/openbot-infra/Cargo.toml || fail "infra Server runtime feature missing"
grep -Fq '    "dep:rustls",' crates/openbot-infra/Cargo.toml || fail "infra Server TLS feature missing rustls"
grep -Fq '    "dep:tokio-rustls",' crates/openbot-infra/Cargo.toml || fail "infra Server TLS feature missing tokio-rustls"
grep -Fq 'server-sso = ["server-runtime", "dep:openssl", "dep:quick-xml", "dep:samael"]' crates/openbot-infra/Cargo.toml || fail "infra Server SSO dependency set drift"
grep -Fq 'openbot-infra = { workspace = true, features = ["server-sso"] }' crates/openbot-server/Cargo.toml || fail "Server no longer opts into SSO"
grep -Fq 'openbot-infra = { workspace = true, optional = true, features = ["desktop-local"] }' crates/openbot-desktop/Cargo.toml || fail "Desktop local infra edge drift"

sf_block=$(awk '/^name = "security-framework"$/{show=1} show{print} show && /^$/{exit}' Cargo.lock)
sfs_block=$(awk '/^name = "security-framework-sys"$/{show=1} show{print} show && /^$/{exit}' Cargo.lock)
grep -Fq 'version = "3.7.0"' <<<"$sf_block" || fail "security-framework lock version drift"
grep -Fq 'checksum = "b7f4bc775c73d9a02cde8bf7b2ec4c9d12743edf609006c7facc23998404cd1d"' <<<"$sf_block" || fail "security-framework checksum drift"
grep -Fq 'version = "2.17.0"' <<<"$sfs_block" || fail "security-framework-sys lock version drift"
grep -Fq 'checksum = "6ce2691df843ecc5d231c0b14ece2acc3efb62c0a398c7e1d875f3983ce020e3"' <<<"$sfs_block" || fail "security-framework-sys checksum drift"

metadata=$(cargo metadata --format-version 1 --locked)
build_scripts=$(jq '[.packages[] | select(.name == "security-framework" or .name == "security-framework-sys") | .targets[].kind[] | select(. == "custom-build")] | length' <<<"$metadata")
[[ "$build_scripts" == "0" ]] || fail "key-store crates unexpectedly gained build.rs"

mac_tree=$(cargo tree -p openbot-desktop --all-features --target aarch64-apple-darwin -e normal --locked)
windows_tree=$(cargo tree -p openbot-desktop --all-features --target x86_64-pc-windows-msvc -e normal --locked)
linux_tree=$(cargo tree -p openbot-desktop --all-features --target x86_64-unknown-linux-gnu -e normal --locked)
server_tree=$(cargo tree -p openbot-server --target aarch64-apple-darwin -e normal --locked)

grep -Fq 'security-framework v3.7.0' <<<"$mac_tree" || fail "macOS Desktop graph lacks Keychain adapter"
if grep -Fq 'security-framework v' <<<"$windows_tree$linux_tree$server_tree"; then
  fail "macOS Security.framework leaked into Windows/Linux/Server graph"
fi
grep -Fq 'openbot-windows-sandbox v' <<<"$windows_tree" || fail "Windows Desktop graph lacks sole unsafe Credential Manager boundary"
if grep -Fq 'openbot-windows-sandbox v' <<<"$mac_tree$linux_tree$server_tree"; then
  fail "Windows unsafe boundary leaked into macOS/Linux/Server graph"
fi
if grep -Eq '(^| )(samael|openssl-sys|ring|rustls) v' <<<"$mac_tree$windows_tree$linux_tree"; then
  fail "Server TLS/SAML/xmlsec/OpenSSL leaked into Desktop graph"
fi
grep -Fq 'samael v0.0.22' <<<"$server_tree" || fail "Server graph lost pinned SAML implementation"
grep -Fq 'openssl-sys v' <<<"$server_tree" || fail "Server graph lost reviewed xmlsec/OpenSSL edge"
grep -Fq 'rustls v' <<<"$server_tree" || fail "Server graph lost reviewed TLS edge"

keychain_consumers=$(rg -l 'security_framework(::|_sys::)' crates --glob '*.rs' | sort || true)
[[ "$keychain_consumers" == "crates/openbot-desktop/src/postgres_sidecar.rs" ]] || fail "Security.framework consumer set drift: ${keychain_consumers:-none}"

credential_consumers=$(rg -l 'Cred(ReadW|WriteW|Free|DeleteW)' crates --glob '*.rs' | sort || true)
[[ "$credential_consumers" == "crates/openbot-windows-sandbox/src/windows.rs" ]] || fail "Credential Manager FFI consumer set drift: ${credential_consumers:-none}"

production_source=$(awk '/^mod tests \{/{exit} {print}' crates/openbot-desktop/src/postgres_sidecar.rs)
if rg -n 'std::env::(var|var_os|vars|vars_os)|std::process::Command|PGPASSWORD|--pwfile' <<<"$production_source" >/dev/null; then
  fail "PostgreSQL secret path gained environment or command fallback"
fi

desktop_bootstrap_source=$(awk '/^mod tests \{/{exit} {print}' crates/openbot-desktop/src/desktop_local_bootstrap.rs)
infra_bootstrap_source=$(awk '/^mod tests \{/{exit} {print}' crates/openbot-infra/src/db/desktop_local.rs)
if rg -n 'DatabaseConfig::new|std::env::(var|var_os|vars|vars_os)' <<<"$desktop_bootstrap_source$infra_bootstrap_source" >/dev/null; then
  fail "Desktop Local bootstrap gained String config or environment fallback"
fi

echo "PostgreSQL key-store dependency guard: ok (macOS Keychain; Windows sole unsafe Cred*; Desktop TLS/SAML/OpenSSL=0; Server runtime+SSO retained; build.rs=0)"
