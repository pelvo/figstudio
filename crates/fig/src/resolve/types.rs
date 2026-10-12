//! Instance path ids and resolver errors.
use std::backtrace::Backtrace;
use std::fmt;

use crate::Guid;

/// A node inside an instance, as Figma names it: `I<instance>;<key>[;<key>...]`. The first
/// guid is the instance in the document; each further one is the override key (`overrideKey`,
/// else the guid) of a node in the component the previous instance shows. Ordinary frames and
/// groups between them are not part of the path, only nested instances.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstancePath(pub(super) Vec<Guid>);

impl InstancePath {
    /// `None` unless there is an instance and at least one key.
    pub fn new(guids: Vec<Guid>) -> Option<Self> {
        (guids.len() >= 2).then_some(Self(guids))
    }

    /// The instance in the document.
    pub fn instance(&self) -> Guid {
        self.0[0]
    }

    /// The override path inside the instance (every guid after the first).
    pub fn keys(&self) -> &[Guid] {
        &self.0[1..]
    }
}

impl fmt::Display for InstancePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("I")?;
        for (index, guid) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(";")?;
            }
            write!(f, "{guid}")?;
        }
        Ok(())
    }
}

/// Why an instance or instance path could not be resolved.
#[derive(Debug)]
pub struct ResolveError {
    kind: Kind,
    backtrace: Backtrace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    UnknownNode,
    NotInstance,
    MissingComponent,
    TooDeep,
    UnknownPath,
}

impl ResolveError {
    pub(super) fn new(kind: Kind) -> Self {
        Self {
            kind,
            backtrace: Backtrace::capture(),
        }
    }

    /// The instance, or a key of the path, names no node at that place.
    pub fn is_not_found(&self) -> bool {
        matches!(self.kind, Kind::UnknownNode | Kind::UnknownPath)
    }

    /// The captured backtrace (empty unless `RUST_BACKTRACE` is set).
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.kind {
            Kind::UnknownNode => "the instance does not exist in this file",
            Kind::NotInstance => "the first id of an instance path must be an instance",
            Kind::MissingComponent => "the instance's component is missing from this file",
            Kind::TooDeep => "instances nest too deeply or form a cycle",
            Kind::UnknownPath => "the instance path names no node inside the instance",
        })
    }
}

impl std::error::Error for ResolveError {}
