//! The executor credential: a 256-bit secret from which the caller room is derived.
//! Pure (no browser types) so native tests cover the format and the derivation vector,
//! which `web/executor-auth.mjs` must reproduce exactly (SHA-256 over
//! `"figstudio/room/v1"`, a zero byte, then the raw 32-byte executor).
use sha2::{Digest, Sha256};

/// Domain separator for the room derivation; ASCII, NUL-terminated in the preimage.
pub(crate) const ROOM_DOMAIN: &[u8] = b"figstudio/room/v1";
/// The constant first WebSocket subprotocol; the raw executor hex is the second.
pub(crate) const BRIDGE_PROTOCOL: &str = "figstudio-exec-v1";

/// An executor secret (`ex_` + 64 lower-case hex) and the caller room derived from it.
/// Only the executor is ever persisted; the room is always re-derived. The secret never
/// appears in a URL path, query, log or error — `Debug` is redacted.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ExecutorPair(String);

impl std::fmt::Debug for ExecutorPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The derived room is public (it names the URL); the executor is the capability.
        write!(
            f,
            "ExecutorPair(executor: <redacted>, room: {})",
            self.room()
        )
    }
}

impl ExecutorPair {
    /// A fresh pair from OS/browser entropy.
    pub(crate) fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes)?;
        Ok(Self(format!("ex_{}", hex(&bytes))))
    }

    /// A stored executor token, if it is exactly `ex_` + 64 lower-case hex.
    pub(crate) fn from_executor(token: &str) -> Option<Self> {
        valid_executor(token).then(|| Self(token.to_owned()))
    }

    /// The executor token (`ex_…`); the secret. Persisted, and put in join links.
    pub(crate) fn executor(&self) -> &str {
        &self.0
    }

    /// The raw executor hex (no prefix); offered only as the second WebSocket subprotocol.
    pub(crate) fn executor_hex(&self) -> &str {
        &self.0[3..]
    }

    /// The caller room (`rm_…`) this executor authorizes; derived, never stored.
    pub(crate) fn room(&self) -> String {
        derive_room(self.executor_hex()).expect("valid executor by construction")
    }

    /// The subprotocols the relay bridge WebSocket offers: marker first, executor hex second.
    pub(crate) fn subprotocols(&self) -> [String; 2] {
        [BRIDGE_PROTOCOL.to_owned(), self.executor_hex().to_owned()]
    }
}

/// `rm_<64 hex>`: SHA-256 over the domain, a zero byte, then the raw 32-byte executor.
pub(crate) fn derive_room(executor_hex: &str) -> Option<String> {
    let mut preimage = Vec::with_capacity(ROOM_DOMAIN.len() + 1 + 32);
    preimage.extend_from_slice(ROOM_DOMAIN);
    preimage.push(0);
    preimage.extend(decode_hex(executor_hex)?);
    Some(format!("rm_{}", hex(&Sha256::digest(&preimage))))
}

pub(crate) fn valid_executor(token: &str) -> bool {
    token.len() == 67
        && token.starts_with("ex_")
        && token[3..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 128-bit random lowercase hex (`32` hex characters) from OS/browser entropy.
/// Never falls back to zeros when the RNG fails.
pub(crate) fn random_hex_16() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    Ok(hex(&bytes))
}

/// A stored device id is valid only when it is 32 lowercase hex characters and not all zeros.
pub(crate) fn valid_device_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && id.bytes().any(|b| b != b'0')
}

/// The `#room=` and `#exec=` keys together: both present, both valid, and the room must
/// be the one derived from that executor. Anything else is not a proposal.
pub(crate) fn pair_from_fragment(fragment: &str) -> Option<ExecutorPair> {
    let fragment = fragment
        .rsplit_once('#')
        .map_or(fragment, |(_, after)| after);
    let mut room = None;
    let mut executor = None;
    for (key, value) in form_urlencoded::parse(fragment.as_bytes()) {
        match key.as_ref() {
            "room" if crate::mcp_wire::valid_room(&value) => room = Some(value.into_owned()),
            "exec" if valid_executor(&value) => executor = Some(value.into_owned()),
            _ => {}
        }
    }
    let executor = ExecutorPair::from_executor(&executor?)?;
    (room? == executor.room()).then_some(executor)
}

/// An executor as typed or pasted: a bare `ex_…` token, or a complete join link whose
/// `room` and `exec` match. A caller MCP URL or bare `rm_…` room is rejected: the caller
/// room alone is not the executor capability.
pub(crate) fn pair_from_input(input: &str) -> Option<ExecutorPair> {
    let input = input.trim();
    if let Some(pair) = pair_from_fragment(input) {
        return Some(pair);
    }
    ExecutorPair::from_executor(input)
}

/// The join link: `https://<host>/#room=<derived>&exec=ex_…`. It carries the executor
/// secret in the fragment, which never reaches a server.
pub(crate) fn pair_link(protocol: &str, host: &str, pair: &ExecutorPair) -> Option<String> {
    match protocol {
        "https:" | "http:" => (),
        _ => return None,
    }
    (!host.is_empty()).then(|| {
        format!(
            "{}//{}#room={}&exec={}",
            protocol,
            host,
            pair.room(),
            pair.executor()
        )
    })
}

/// What loading found in storage, and what to do about it. Pure: `mcp_room.rs` applies it,
/// in the order the plan requires (save the new executor before deleting the legacy key).
pub(crate) enum Load {
    /// No v2 executor stored: mint and save a fresh pair.
    Fresh,
    /// A valid v2 executor: use it. `legacy` says a legacy room key lingers to delete.
    Use { pair: ExecutorPair, legacy: bool },
    /// No v2 executor, but a legacy caller room exists: mint a NEW pair (a caller-only
    /// token is never adopted as an executor), save it, then delete the legacy key.
    Upgrade,
    /// A stored v2 value that is not a valid executor: fail visibly; the legacy caller
    /// room is not a fallback.
    Corrupt,
}

pub(crate) fn plan(v2: Option<String>, legacy: Option<String>) -> Load {
    match v2 {
        Some(stored) => match ExecutorPair::from_executor(&stored) {
            Some(pair) => Load::Use {
                pair,
                legacy: legacy.is_some(),
            },
            None => Load::Corrupt,
        },
        None if legacy.is_some() => Load::Upgrade,
        None => Load::Fresh,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(hex: &str) -> Option<[u8; 32]> {
    let bytes = hex.as_bytes();
    if bytes.len() != 64 || !bytes.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
#[path = "mcp_pair_tests.rs"]
mod tests;
