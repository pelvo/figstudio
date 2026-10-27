//! Build provenance for update downloads. Before the app zip is unpacked, GitHub must hold an
//! artifact attestation for its SHA-256 (a Sigstore bundle, made by `actions/attest` in the
//! release workflow) whose signing certificate GitHub Actions issued to figstudio's release
//! workflow on `main`. Uploading a release asset is then not enough to ship code to every
//! install: the asset must also have been built and signed by that workflow.
//!
//! The bundle is checked by Sigstore's verifier (`sigstore-verify`) against its built-in
//! production trust root: the certificate chain to Fulcio, the transparency-log entry and
//! the signature, and that the attested subject is exactly this digest.

use std::io::Read;
use std::time::Duration;

use anyhow::{Context as _, bail};
use sigstore_verify::trust_root::{SIGSTORE_PRODUCTION_TRUSTED_ROOT, TrustedRoot};
use sigstore_verify::types::{Artifact, Bundle, Sha256Hash};
use sigstore_verify::{IdentityMatcher, VerificationPolicy, Verifier};

/// The certificate identity (SAN) of figstudio's release workflow running on `main`.
pub const WORKFLOW: &str =
    "https://github.com/pelvo/figstudio/.github/workflows/release-please.yml@refs/heads/main";
/// GitHub Actions' OIDC issuer.
const ISSUER: &str = "https://token.actions.githubusercontent.com";
/// The attestations GitHub holds for a digest of this repository's artifacts.
const ATTESTATIONS: &str = "https://api.github.com/repos/pelvo/figstudio/attestations/sha256:";
/// A bundle is a few kilobytes; anything far larger is not one.
const MAX_BUNDLE: usize = 1 << 20;
/// At most this many attestations are fetched for one digest.
const MAX_ATTESTATIONS: usize = 16;

#[derive(serde::Deserialize)]
struct List {
    #[serde(default)]
    attestations: Vec<Entry>,
}

#[derive(serde::Deserialize)]
struct Entry {
    #[serde(default)]
    bundle: Option<serde_json::Value>,
    #[serde(default)]
    bundle_url: Option<String>,
}

/// Fetches the attestations GitHub holds for the artifact with SHA-256 `sha256` (lower-case
/// hex) and checks that one verifies for [`WORKFLOW`].
pub fn check(http: &reqwest::blocking::Client, sha256: &str) -> anyhow::Result<()> {
    let reply = http
        .get(format!("{ATTESTATIONS}{sha256}"))
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(60))
        .send()
        .context("could not reach GitHub for the build attestation")?;
    if reply.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("this release has no build attestation");
    }
    let text = reply
        .error_for_status()
        .and_then(reqwest::blocking::Response::text)
        .context("could not read the build attestations")?;
    let list: List = serde_json::from_str(&text).context("unexpected attestation reply")?;
    let mut bundles = Vec::new();
    for entry in list.attestations.into_iter().take(MAX_ATTESTATIONS) {
        match (entry.bundle, entry.bundle_url) {
            (Some(bundle), _) if !bundle.is_null() => bundles.push(bundle.to_string()),
            (_, Some(url)) => bundles.push(fetch_bundle(http, &url)?),
            _ => {}
        }
    }
    attested(&bundles, sha256, WORKFLOW)
}

/// A bundle from its `bundle_url`: raw Snappy-compressed JSON (or plain JSON).
fn fetch_bundle(http: &reqwest::blocking::Client, url: &str) -> anyhow::Result<String> {
    let reply = http
        .get(url)
        .timeout(Duration::from_secs(60))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .context("could not download the build attestation")?;
    let mut raw = Vec::new();
    reply
        .take(MAX_BUNDLE as u64 + 1)
        .read_to_end(&mut raw)
        .context("could not download the build attestation")?;
    bundle_json(&raw)
}

/// The bundle's JSON from what `bundle_url` serves.
fn bundle_json(raw: &[u8]) -> anyhow::Result<String> {
    if raw.len() > MAX_BUNDLE {
        bail!("the build attestation is too large");
    }
    let json = if raw.first() == Some(&b'{') {
        raw.to_vec()
    } else {
        let len = snap::raw::decompress_len(raw).context("unreadable build attestation")?;
        if len > MAX_BUNDLE {
            bail!("the build attestation is too large");
        }
        snap::raw::Decoder::new()
            .decompress_vec(raw)
            .context("unreadable build attestation")?
    };
    String::from_utf8(json).context("unreadable build attestation")
}

/// Whether one of `bundles` (Sigstore bundle JSON) attests the artifact with SHA-256
/// `sha256` (hex), signed with a GitHub Actions certificate for `identity`.
pub fn attested(bundles: &[String], sha256: &str, identity: &str) -> anyhow::Result<()> {
    if bundles.is_empty() {
        bail!("this release has no build attestation");
    }
    let root = TrustedRoot::from_json(SIGSTORE_PRODUCTION_TRUSTED_ROOT)
        .context("Sigstore's trust root does not load")?;
    let verifier = Verifier::new(&root).context("Sigstore's trust root does not load")?;
    let digest = Sha256Hash::from_hex(sha256).context("not a SHA-256")?;
    let policy = VerificationPolicy::new(IdentityMatcher::Uri(identity.to_owned()), ISSUER);
    let mut last = None;
    for json in bundles {
        let verified = Bundle::from_json(json)
            .map_err(anyhow::Error::from)
            .and_then(|bundle| {
                verifier
                    .verify(Artifact::from_digest(digest), &bundle, &policy)
                    .map_err(anyhow::Error::from)
            });
        match verified {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    let why = last.map_or_else(String::new, |e| format!(": {e:#}"));
    bail!("no build attestation from figstudio's release workflow verifies{why}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real, public GitHub build attestation: the SLSA provenance `actions/attest` made
    /// for the GitHub CLI's v2.103.3 release files, `gh_2.103.3_linux_amd64.deb` among
    /// them (cli/cli, MIT), as GitHub's attestation API served it (decompressed).
    const GH_BUNDLE: &str = include_str!("update_attest_gh_bundle.json");
    const GH_DEB: &str = "e3bafef96328d1a67de3a968bdfe1cc2ffd58cf6aba8b5e201356897ed06bb3d";
    const GH_WORKFLOW: &str =
        "https://github.com/cli/cli/.github/workflows/deployment.yml@refs/heads/trunk";

    #[test]
    fn a_real_attestation_verifies_for_its_workflow_and_digest_only() {
        let bundles = [GH_BUNDLE.to_owned()];
        assert!(attested(&bundles, GH_DEB, GH_WORKFLOW).is_ok());
        // Another workflow's signature is not figstudio's release workflow.
        assert!(attested(&bundles, GH_DEB, WORKFLOW).is_err());
        // The attestation is for that file, not for any download.
        let other = "0".repeat(64);
        assert!(attested(&bundles, &other, GH_WORKFLOW).is_err());
    }

    #[test]
    fn a_tampered_or_missing_attestation_is_refused() {
        assert!(attested(&[], GH_DEB, GH_WORKFLOW).is_err(), "none");
        assert!(attested(&["{}".to_owned()], GH_DEB, GH_WORKFLOW).is_err());
        // The signed statement's subject changed by one character.
        let tampered = GH_BUNDLE.replacen("\"payload\":\"", "\"payload\":\"e", 1);
        assert_ne!(tampered, GH_BUNDLE);
        assert!(attested(&[tampered], GH_DEB, GH_WORKFLOW).is_err());
        // A good one among bad ones is enough.
        let mixed = ["{}".to_owned(), GH_BUNDLE.to_owned()];
        assert!(attested(&mixed, GH_DEB, GH_WORKFLOW).is_ok());
    }

    #[test]
    fn bundles_are_read_compressed_or_plain_and_size_capped() -> anyhow::Result<()> {
        let compressed = snap::raw::Encoder::new().compress_vec(GH_BUNDLE.as_bytes())?;
        assert_eq!(bundle_json(&compressed)?, GH_BUNDLE);
        assert_eq!(bundle_json(GH_BUNDLE.as_bytes())?, GH_BUNDLE);
        assert!(bundle_json(&vec![b'{'; MAX_BUNDLE + 1]).is_err());
        assert!(bundle_json(b"\xff\xff\xff\xff\x0f not snappy").is_err());
        Ok(())
    }
}
