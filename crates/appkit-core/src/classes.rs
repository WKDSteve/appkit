//! The data classes of what a hub holds, and which may go to a phone (`MOB-02`).
//!
//! The class of an area, a repository or an item is the person's statement on the PC (an app keeps it in its settings,
//! tasks-desktop in `areas.json`), never something the data says about itself. **Fail closed:** what has no class, or
//! one this code does not know, is `Unclassified`, and neither `Private` nor `Unclassified` ever goes to a phone: not in
//! an answer, not into its cache, not in a push. A phone is its own channel, and a class is released for it only when
//! the class says so here.
//!
//! What is made from several things (a count, a list of blockers, a slug of a title, an id in an edge) has the highest
//! class of them ([`highest`]).

use serde::{Deserialize, Serialize};

/// The class of a piece of data, from the least to the most guarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataClass {
    /// May leave the hub, also into a public report.
    Public,
    /// The person's own and not secret: another panel of the same person, and a paired phone.
    Internal,
    /// Nobody said: treated as never leaving the hub.
    Unclassified,
    /// Never leaves the hub.
    Private,
}

impl DataClass {
    /// The class a setting names; anything else (missing, misspelt, another case) is `Unclassified`.
    #[must_use]
    pub fn parse(word: Option<&str>) -> Self {
        match word {
            Some("public") => Self::Public,
            Some("internal") => Self::Internal,
            Some("private") => Self::Private,
            _ => Self::Unclassified,
        }
    }

    /// May data of this class go to a paired phone (in an answer, into its cache, in a push)? Only `public` and
    /// `internal`.
    #[must_use]
    pub fn released_for_device(self) -> bool {
        matches!(self, Self::Public | Self::Internal)
    }
}

/// The class of something made from data of these classes: the highest of them; nothing at all is `Unclassified`.
#[must_use]
pub fn highest(classes: impl IntoIterator<Item = DataClass>) -> DataClass {
    classes.into_iter().max().unwrap_or(DataClass::Unclassified)
}

/// Who asks: the person on the PC (a session of a pairing code), or a paired device by its id. A hub's route reads it
/// to cut its answer for a device before it is sent; the guard sets it for every route below `/api/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    Pc,
    Device(String),
}

impl Caller {
    /// May this caller be given data of `class`? The PC is given everything it may see anyway; a device only what is
    /// released for it.
    #[must_use]
    pub fn may_see(&self, class: DataClass) -> bool {
        match self {
            Self::Pc => true,
            Self::Device(_) => class.released_for_device(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_and_internal_go_to_a_phone_and_anything_unknown_is_unclassified() {
        assert!(DataClass::Public.released_for_device());
        assert!(DataClass::Internal.released_for_device());
        assert!(!DataClass::Private.released_for_device());
        assert!(!DataClass::Unclassified.released_for_device());
        for word in [
            None,
            Some(""),
            Some("Public"),
            Some("secret"),
            Some("internal "),
        ] {
            assert_eq!(DataClass::parse(word), DataClass::Unclassified, "{word:?}");
        }
        assert_eq!(DataClass::parse(Some("private")), DataClass::Private);
    }

    #[test]
    fn what_is_made_from_several_has_the_highest_class() {
        use DataClass::*;
        assert_eq!(highest([Public, Internal]), Internal);
        assert_eq!(highest([Public, Unclassified]), Unclassified);
        assert_eq!(highest([Internal, Private, Public]), Private);
        assert_eq!(highest([]), Unclassified);
        assert!(!highest([Public, Private]).released_for_device());
    }

    #[test]
    fn a_device_sees_only_released_classes_and_the_pc_everything() {
        let phone = Caller::Device("ab".repeat(16));
        assert!(phone.may_see(DataClass::Internal));
        assert!(!phone.may_see(DataClass::Private));
        assert!(!phone.may_see(DataClass::Unclassified));
        assert!(Caller::Pc.may_see(DataClass::Private));
    }
}
