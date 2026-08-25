#!/usr/bin/env bash
# G4 Batch 12: MCP OAuth must stay on SafeDialer, exact issuer/resource and local-first revocation.
set -euo pipefail

fail() {
  printf 'MCP OAuth runtime guard: FAIL: %s\n' "$1" >&2
  exit 1
}

files=(
  crates/openbot-infra/src/mcp_oauth.rs
  crates/openbot-infra/src/mcp_connections.rs
  crates/openbot-infra/src/mcp_credentials.rs
)

if rg -n 'reqwest|hyper::|TcpStream|lookup_host|Command::new|std::process' "${files[@]}"; then
  fail 'OAuth/credential code bypasses the unique SafeDialer or starts a process'
fi
grep -qF 'SafeHttpRequest::mcp(' crates/openbot-infra/src/mcp_oauth.rs \
  || fail '401 protected-resource probe no longer uses bounded SafeDialer MCP request'
grep -qF 'bearer_parameter(challenge, "resource_metadata")' crates/openbot-infra/src/mcp_oauth.rs \
  || fail 'WWW-Authenticate resource_metadata priority disappeared'
grep -qF 'serializer.append_pair("resource", resource);' crates/openbot-infra/src/mcp_oauth.rs \
  || fail 'authorization/code token resource binding disappeared'
grep -qF 'serializer.append_pair("resource", discovery.resource());' crates/openbot-infra/src/mcp_oauth.rs \
  || fail 'refresh token resource binding disappeared'
grep -qF 'code_challenge_method", "S256"' crates/openbot-infra/src/mcp_oauth.rs \
  || fail 'PKCE S256 authorization binding disappeared'
grep -qF 'metadata.issuer != client.issuer' crates/openbot-infra/src/mcp_oauth.rs \
  || fail 'authorization-server exact issuer validation disappeared'

grep -qF 'openbot-mcp-oauth-state-v1' crates/openbot-infra/src/mcp_connections.rs \
  || fail 'OAuth state HMAC purpose separation disappeared'
grep -qF 'openbot-mcp-oauth-attempt-aead-v1' crates/openbot-infra/src/mcp_connections.rs \
  || fail 'OAuth attempt AEAD purpose separation disappeared'
grep -qF 'DELETE FROM public.verifications WHERE identifier=$1' crates/openbot-infra/src/mcp_connections.rs \
  || fail 'callback no longer burns state before validation/network'
grep -qF "FOR UPDATE SKIP LOCKED" crates/openbot-infra/src/mcp_connections.rs \
  || fail 'pending vendor revocation is no longer multi-replica claimed'
grep -qF "'revocation_status','pending'" crates/openbot-infra/src/mcp_connections.rs \
  || fail 'local-first disconnect tombstone disappeared'

grep -qF 'ADD COLUMN credential_generation bigint' crates/openbot-infra/sql/native_0018.sql \
  || fail 'credential generation migration disappeared'
grep -qF 'g.credential_generation=coalesce(s.credential_generation,0)' crates/openbot-infra/src/mcp_catalog.rs \
  || fail 'grant visibility no longer binds deployment credential generation'
grep -qF 'outcome == Err(McpClientError::AuthRequired)' crates/openbot-infra/src/agent_tools.rs \
  || fail 'controlled OAuth 401 refresh/retry branch disappeared'

authorization_slice=$(sed -n '/pub async fn authorization_plan/,/pub async fn exchange_authorization_code/p' crates/openbot-infra/src/mcp_oauth.rs)
if rg -n 'append_pair\("(access_token|refresh_token|client_secret)"' <<< "$authorization_slice"; then
  fail 'a credential was added to an authorization URL query'
fi
grep -qF '.field("client_secret", &"[redacted]")' crates/openbot-contracts/src/mcp.rs \
  || fail 'admin OAuth client Debug redaction disappeared'

printf 'MCP OAuth runtime guard: ok (SafeDialer PRM/issuer/resource; HMAC+AEAD state; credential generation; local-first revoke)\n'
