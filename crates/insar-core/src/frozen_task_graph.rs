use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::{
    AssetRole, FrozenTaskSnapshotV3, FrozenTransferOrigin, OrbitPlatform, OrbitProductType,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrozenTaskSnapshotV3GraphErrorKind {
    DuplicateIdentity,
    MissingReference,
    InvalidOrbitMatch,
    InvalidTransferGraph,
    InvalidCompanionCoverage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrozenTaskSnapshotV3GraphIdentity {
    SourceItemId,
    TransferItemId,
    SelectedItemId,
    TransferSourceItemId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrozenTaskSnapshotV3GraphReference {
    InitialSelectedSource,
    TransferSource,
    AssociationSource,
    AssociationOrbitTransfer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrozenTaskSnapshotV3GraphLocation {
    SourceItem {
        index: usize,
    },
    InitialSelectedItemId {
        index: usize,
    },
    TransferItem {
        index: usize,
    },
    TransferSourceItem {
        transfer_index: usize,
        source_index: usize,
    },
    OrbitAssociation {
        index: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrozenTaskSnapshotV3OrbitIssue {
    RequestedOrbitTypeNotPoeorb,
    MatchedOrbitTypeNotPoeorb,
    UnknownSourcePlatform,
    PlatformMismatch,
    InvalidSourceInterval,
    ValidityNotCovered,
    FileNameMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrozenTaskSnapshotV3TransferGraphIssue {
    SelectedSourceSetMismatch,
    SelectedProductCoverage {
        source: FrozenTaskSnapshotV3GraphLocation,
        primary_transfer_count: usize,
    },
    SelectedProductSourceCount {
        transfer: FrozenTaskSnapshotV3GraphLocation,
        source_count: usize,
    },
    SelectedProductReferenceMismatch {
        transfer: FrozenTaskSnapshotV3GraphLocation,
        source: FrozenTaskSnapshotV3GraphLocation,
    },
    SelectedProductRoleMustBePrimary {
        transfer: FrozenTaskSnapshotV3GraphLocation,
    },
    RequiredCompanionRoleMustBeAuxiliary {
        transfer: FrozenTaskSnapshotV3GraphLocation,
    },
    RequiredCompanionMustBeRequired {
        transfer: FrozenTaskSnapshotV3GraphLocation,
    },
    AssociationTargetsNonCompanion {
        association: FrozenTaskSnapshotV3GraphLocation,
        transfer: FrozenTaskSnapshotV3GraphLocation,
    },
    AttachmentAssociationSetMismatch {
        transfer: FrozenTaskSnapshotV3GraphLocation,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrozenTaskSnapshotV3CompanionCoverageSubject {
    SourceItem {
        source: FrozenTaskSnapshotV3GraphLocation,
    },
    RequiredCompanionTransfer {
        transfer: FrozenTaskSnapshotV3GraphLocation,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrozenTaskSnapshotV3GraphError {
    DuplicateIdentity {
        identity: FrozenTaskSnapshotV3GraphIdentity,
        first: FrozenTaskSnapshotV3GraphLocation,
        duplicate: FrozenTaskSnapshotV3GraphLocation,
    },
    MissingReference {
        reference: FrozenTaskSnapshotV3GraphReference,
        at: FrozenTaskSnapshotV3GraphLocation,
    },
    InvalidOrbitMatch {
        association: FrozenTaskSnapshotV3GraphLocation,
        issue: FrozenTaskSnapshotV3OrbitIssue,
    },
    InvalidTransferGraph {
        issue: FrozenTaskSnapshotV3TransferGraphIssue,
    },
    InvalidCompanionCoverage {
        subject: FrozenTaskSnapshotV3CompanionCoverageSubject,
        association_count: usize,
    },
}

impl FrozenTaskSnapshotV3GraphError {
    pub fn kind(&self) -> FrozenTaskSnapshotV3GraphErrorKind {
        match self {
            Self::DuplicateIdentity { .. } => FrozenTaskSnapshotV3GraphErrorKind::DuplicateIdentity,
            Self::MissingReference { .. } => FrozenTaskSnapshotV3GraphErrorKind::MissingReference,
            Self::InvalidOrbitMatch { .. } => FrozenTaskSnapshotV3GraphErrorKind::InvalidOrbitMatch,
            Self::InvalidTransferGraph { .. } => {
                FrozenTaskSnapshotV3GraphErrorKind::InvalidTransferGraph
            }
            Self::InvalidCompanionCoverage { .. } => {
                FrozenTaskSnapshotV3GraphErrorKind::InvalidCompanionCoverage
            }
        }
    }
}

impl fmt::Display for FrozenTaskSnapshotV3GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind() {
            FrozenTaskSnapshotV3GraphErrorKind::DuplicateIdentity => "duplicate identity",
            FrozenTaskSnapshotV3GraphErrorKind::MissingReference => "missing reference",
            FrozenTaskSnapshotV3GraphErrorKind::InvalidOrbitMatch => "invalid orbit match",
            FrozenTaskSnapshotV3GraphErrorKind::InvalidTransferGraph => "invalid transfer graph",
            FrozenTaskSnapshotV3GraphErrorKind::InvalidCompanionCoverage => {
                "invalid companion coverage"
            }
        };
        write!(
            formatter,
            "frozen V3 task graph validation failed: {reason}"
        )
    }
}

impl std::error::Error for FrozenTaskSnapshotV3GraphError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrozenTaskSnapshotV3ValidationError {
    Structural,
    Graph(FrozenTaskSnapshotV3GraphError),
}

impl fmt::Display for FrozenTaskSnapshotV3ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Structural => {
                write!(
                    formatter,
                    "frozen V3 task snapshot structural validation failed"
                )
            }
            Self::Graph(_) => write!(formatter, "frozen V3 task snapshot graph validation failed"),
        }
    }
}

impl std::error::Error for FrozenTaskSnapshotV3ValidationError {}

pub fn validate_frozen_task_snapshot_v3(
    snapshot: &FrozenTaskSnapshotV3,
) -> Result<(), FrozenTaskSnapshotV3ValidationError> {
    snapshot
        .validate()
        .map_err(|_| FrozenTaskSnapshotV3ValidationError::Structural)?;
    validate_frozen_task_snapshot_v3_graph(snapshot)
        .map_err(FrozenTaskSnapshotV3ValidationError::Graph)
}

/// Validates only cross-object V3 task graph invariants.
///
/// # Preconditions
///
/// Call FrozenTaskSnapshotV3::validate() first. This graph-only function assumes
/// generated structural validation has already accepted the object shape.
pub fn validate_frozen_task_snapshot_v3_graph(
    snapshot: &FrozenTaskSnapshotV3,
) -> Result<(), FrozenTaskSnapshotV3GraphError> {
    let mut source_by_id = BTreeMap::new();
    for (source_index, source) in snapshot.source_items.iter().enumerate() {
        let source_id = source.item_id.as_str();
        let location = FrozenTaskSnapshotV3GraphLocation::SourceItem {
            index: source_index,
        };
        if let Some((first_index, _)) = source_by_id.get(source_id) {
            return Err(FrozenTaskSnapshotV3GraphError::DuplicateIdentity {
                identity: FrozenTaskSnapshotV3GraphIdentity::SourceItemId,
                first: FrozenTaskSnapshotV3GraphLocation::SourceItem {
                    index: *first_index,
                },
                duplicate: location,
            });
        }
        source_by_id.insert(source_id, (source_index, source));
    }

    let mut selected_ids = BTreeMap::new();
    for (selected_index, source_id) in snapshot.initial_selected_item_ids.iter().enumerate() {
        let location = FrozenTaskSnapshotV3GraphLocation::InitialSelectedItemId {
            index: selected_index,
        };
        if let Some(first_index) = selected_ids.insert(source_id.as_str(), selected_index) {
            return Err(FrozenTaskSnapshotV3GraphError::DuplicateIdentity {
                identity: FrozenTaskSnapshotV3GraphIdentity::SelectedItemId,
                first: FrozenTaskSnapshotV3GraphLocation::InitialSelectedItemId {
                    index: first_index,
                },
                duplicate: location,
            });
        }
        if !source_by_id.contains_key(source_id.as_str()) {
            return Err(FrozenTaskSnapshotV3GraphError::MissingReference {
                reference: FrozenTaskSnapshotV3GraphReference::InitialSelectedSource,
                at: location,
            });
        }
    }

    let source_ids: BTreeSet<_> = source_by_id.keys().copied().collect();
    let selected_id_set: BTreeSet<_> = selected_ids.keys().copied().collect();
    if source_ids != selected_id_set {
        return Err(FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
            issue: FrozenTaskSnapshotV3TransferGraphIssue::SelectedSourceSetMismatch,
        });
    }

    let mut transfer_by_id = BTreeMap::new();
    for (transfer_index, transfer) in snapshot.transfer_items.iter().enumerate() {
        let transfer_id = transfer.transfer_item_id.as_str();
        let location = FrozenTaskSnapshotV3GraphLocation::TransferItem {
            index: transfer_index,
        };
        if let Some((first_index, _)) = transfer_by_id.get(transfer_id) {
            return Err(FrozenTaskSnapshotV3GraphError::DuplicateIdentity {
                identity: FrozenTaskSnapshotV3GraphIdentity::TransferItemId,
                first: FrozenTaskSnapshotV3GraphLocation::TransferItem {
                    index: *first_index,
                },
                duplicate: location,
            });
        }
        transfer_by_id.insert(transfer_id, (transfer_index, transfer));
    }

    let mut primary_transfer_count = BTreeMap::new();
    let mut companion_attachment_sets = BTreeMap::new();
    for (transfer_index, transfer) in snapshot.transfer_items.iter().enumerate() {
        let transfer_location = FrozenTaskSnapshotV3GraphLocation::TransferItem {
            index: transfer_index,
        };
        let mut attachment_ids = BTreeMap::new();
        for (source_index, source_id) in transfer.source_item_ids.iter().enumerate() {
            let location = FrozenTaskSnapshotV3GraphLocation::TransferSourceItem {
                transfer_index,
                source_index,
            };
            if let Some(first_index) = attachment_ids.insert(source_id.as_str(), source_index) {
                return Err(FrozenTaskSnapshotV3GraphError::DuplicateIdentity {
                    identity: FrozenTaskSnapshotV3GraphIdentity::TransferSourceItemId,
                    first: FrozenTaskSnapshotV3GraphLocation::TransferSourceItem {
                        transfer_index,
                        source_index: first_index,
                    },
                    duplicate: location,
                });
            }
            if !source_by_id.contains_key(source_id.as_str()) {
                return Err(FrozenTaskSnapshotV3GraphError::MissingReference {
                    reference: FrozenTaskSnapshotV3GraphReference::TransferSource,
                    at: location,
                });
            }
        }
        let attachment_id_set = attachment_ids.keys().copied().collect::<BTreeSet<_>>();

        match &transfer.origin {
            FrozenTransferOrigin::SelectedProduct => {
                if transfer.asset_access.role != AssetRole::Primary {
                    return Err(FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
                        issue:
                            FrozenTaskSnapshotV3TransferGraphIssue::SelectedProductRoleMustBePrimary {
                                transfer: transfer_location,
                            },
                    });
                }
                if transfer.source_item_ids.len() != 1 {
                    return Err(FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
                        issue: FrozenTaskSnapshotV3TransferGraphIssue::SelectedProductSourceCount {
                            transfer: transfer_location,
                            source_count: transfer.source_item_ids.len(),
                        },
                    });
                }
                let source_id = transfer.source_item_ids[0].as_str();
                let (source_index, source) = source_by_id
                    .get(source_id)
                    .expect("transfer source existence is checked above");
                if transfer.asset_access.reference != source.asset_access.reference {
                    return Err(FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
                        issue:
                            FrozenTaskSnapshotV3TransferGraphIssue::SelectedProductReferenceMismatch {
                                transfer: transfer_location,
                                source: FrozenTaskSnapshotV3GraphLocation::SourceItem {
                                    index: *source_index,
                                },
                            },
                    });
                }
                *primary_transfer_count.entry(source_id).or_insert(0) += 1;
            }
            FrozenTransferOrigin::RequiredCompanion => {
                if transfer.asset_access.role != AssetRole::Auxiliary {
                    return Err(FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
                        issue:
                            FrozenTaskSnapshotV3TransferGraphIssue::RequiredCompanionRoleMustBeAuxiliary {
                                transfer: transfer_location,
                            },
                    });
                }
                if !transfer.required {
                    return Err(FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
                        issue:
                            FrozenTaskSnapshotV3TransferGraphIssue::RequiredCompanionMustBeRequired {
                                transfer: transfer_location,
                            },
                    });
                }
                companion_attachment_sets.insert(
                    transfer.transfer_item_id.as_str(),
                    (transfer_index, attachment_id_set),
                );
            }
        }
    }

    for (source_id, (source_index, _)) in &source_by_id {
        let count = primary_transfer_count.get(source_id).copied().unwrap_or(0);
        if count != 1 {
            return Err(FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
                issue: FrozenTaskSnapshotV3TransferGraphIssue::SelectedProductCoverage {
                    source: FrozenTaskSnapshotV3GraphLocation::SourceItem {
                        index: *source_index,
                    },
                    primary_transfer_count: count,
                },
            });
        }
    }

    let mut association_count_by_source = BTreeMap::new();
    let mut association_source_sets = BTreeMap::new();
    for (association_index, association) in snapshot.orbit_associations.iter().enumerate() {
        let association_location = FrozenTaskSnapshotV3GraphLocation::OrbitAssociation {
            index: association_index,
        };
        let source_id = association.source_item_id.as_str();
        if !source_by_id.contains_key(source_id) {
            return Err(FrozenTaskSnapshotV3GraphError::MissingReference {
                reference: FrozenTaskSnapshotV3GraphReference::AssociationSource,
                at: association_location,
            });
        }
        let transfer_id = association.orbit_transfer_item_id.as_str();
        let (transfer_index, transfer) = transfer_by_id.get(transfer_id).ok_or(
            FrozenTaskSnapshotV3GraphError::MissingReference {
                reference: FrozenTaskSnapshotV3GraphReference::AssociationOrbitTransfer,
                at: association_location,
            },
        )?;
        let transfer_location = FrozenTaskSnapshotV3GraphLocation::TransferItem {
            index: *transfer_index,
        };
        if transfer.origin != FrozenTransferOrigin::RequiredCompanion {
            return Err(FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
                issue: FrozenTaskSnapshotV3TransferGraphIssue::AssociationTargetsNonCompanion {
                    association: association_location,
                    transfer: transfer_location,
                },
            });
        }

        let orbit_issue = if association.requested_orbit_type != "POEORB" {
            Some(FrozenTaskSnapshotV3OrbitIssue::RequestedOrbitTypeNotPoeorb)
        } else if association.matched_orbit_metadata.orbit_type != OrbitProductType::POEORB {
            Some(FrozenTaskSnapshotV3OrbitIssue::MatchedOrbitTypeNotPoeorb)
        } else if association.source_platform == OrbitPlatform::UNKNOWN {
            Some(FrozenTaskSnapshotV3OrbitIssue::UnknownSourcePlatform)
        } else if association.source_platform != association.matched_orbit_metadata.platform {
            Some(FrozenTaskSnapshotV3OrbitIssue::PlatformMismatch)
        } else if association.source_acquisition_start > association.source_acquisition_stop {
            Some(FrozenTaskSnapshotV3OrbitIssue::InvalidSourceInterval)
        } else if association
            .matched_orbit_metadata
            .validity_start_unix_seconds
            > association.source_acquisition_start
            || association.source_acquisition_stop
                > association
                    .matched_orbit_metadata
                    .validity_stop_unix_seconds
        {
            Some(FrozenTaskSnapshotV3OrbitIssue::ValidityNotCovered)
        } else if association.matched_orbit_metadata.file_name != transfer.asset_access.file_name {
            Some(FrozenTaskSnapshotV3OrbitIssue::FileNameMismatch)
        } else {
            None
        };
        if let Some(issue) = orbit_issue {
            return Err(FrozenTaskSnapshotV3GraphError::InvalidOrbitMatch {
                association: association_location,
                issue,
            });
        }

        *association_count_by_source.entry(source_id).or_insert(0) += 1;
        association_source_sets
            .entry(transfer_id)
            .or_insert_with(BTreeSet::new)
            .insert(source_id);
    }

    for (transfer_id, (transfer_index, attachment_ids)) in &companion_attachment_sets {
        let transfer_location = FrozenTaskSnapshotV3GraphLocation::TransferItem {
            index: *transfer_index,
        };
        let Some(association_ids) = association_source_sets.get(transfer_id) else {
            return Err(FrozenTaskSnapshotV3GraphError::InvalidCompanionCoverage {
                subject: FrozenTaskSnapshotV3CompanionCoverageSubject::RequiredCompanionTransfer {
                    transfer: transfer_location,
                },
                association_count: 0,
            });
        };
        if attachment_ids != association_ids {
            return Err(FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
                issue: FrozenTaskSnapshotV3TransferGraphIssue::AttachmentAssociationSetMismatch {
                    transfer: transfer_location,
                },
            });
        }
    }

    for (source_id, (source_index, _)) in source_by_id {
        let count = association_count_by_source
            .get(source_id)
            .copied()
            .unwrap_or(0);
        if count != 1 {
            return Err(FrozenTaskSnapshotV3GraphError::InvalidCompanionCoverage {
                subject: FrozenTaskSnapshotV3CompanionCoverageSubject::SourceItem {
                    source: FrozenTaskSnapshotV3GraphLocation::SourceItem {
                        index: source_index,
                    },
                },
                association_count: count,
            });
        }
    }

    Ok(())
}
