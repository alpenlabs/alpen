//! Prover task formatting implementations.

use serde::Serialize;
use strata_paas::{TaskRecordData, TaskStatus};

use super::{helpers::porcelain_field, traits::Formattable};

/// Compact status descriptor used for both JSON and porcelain output.
///
/// Borrows the `TaskStatus` variant name plus the human-relevant fields
/// (retry count, error string) into a single shape so the consumer
/// doesn't need to switch on the enum.
#[derive(Serialize)]
pub(crate) struct StatusInfo {
    pub(crate) name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retry_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

impl From<&TaskStatus> for StatusInfo {
    fn from(status: &TaskStatus) -> Self {
        match status {
            TaskStatus::Pending => Self {
                name: "pending",
                retry_count: None,
                error: None,
            },
            TaskStatus::Proving { retry_count } => Self {
                name: "proving",
                retry_count: Some(*retry_count),
                error: None,
            },
            TaskStatus::Completed => Self {
                name: "completed",
                retry_count: None,
                error: None,
            },
            TaskStatus::TransientFailure { retry_count, error } => Self {
                name: "transient_failure",
                retry_count: Some(*retry_count),
                error: Some(error.clone()),
            },
            TaskStatus::PermanentFailure { error } => Self {
                name: "permanent_failure",
                retry_count: None,
                error: Some(error.clone()),
            },
        }
    }
}

/// Per-task detail emitted by `get-prover-task` and as an element of the
/// summary command's `entries` list.
#[derive(Serialize)]
pub(crate) struct ProverTaskInfo {
    pub(crate) key_hex: String,
    pub(crate) status: StatusInfo,
    pub(crate) updated_at_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retry_after_secs: Option<u64>,
    pub(crate) metadata_len: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata_hex: Option<String>,
}

impl ProverTaskInfo {
    pub(crate) fn from_record(key: &[u8], data: &TaskRecordData) -> Self {
        let metadata = data.metadata();
        Self {
            key_hex: hex::encode(key),
            status: StatusInfo::from(data.status()),
            updated_at_secs: data.updated_at_secs(),
            retry_after_secs: data.retry_after_secs(),
            metadata_len: metadata.map(|m| m.len()).unwrap_or(0),
            metadata_hex: metadata.map(hex::encode),
        }
    }
}

impl Formattable for ProverTaskInfo {
    fn format_porcelain(&self) -> String {
        let mut out = Vec::new();
        out.push(porcelain_field("key_hex", &self.key_hex));
        out.push(porcelain_field("status", self.status.name));
        if let Some(rc) = self.status.retry_count {
            out.push(porcelain_field("status.retry_count", rc));
        }
        if let Some(err) = &self.status.error {
            out.push(porcelain_field("status.error", err));
        }
        out.push(porcelain_field("updated_at_secs", self.updated_at_secs));
        if let Some(when) = self.retry_after_secs {
            out.push(porcelain_field("retry_after_secs", when));
        }
        out.push(porcelain_field("metadata_len", self.metadata_len));
        if let Some(meta) = &self.metadata_hex {
            out.push(porcelain_field("metadata_hex", meta));
        }
        out.join("\n")
    }
}

/// Aggregate counts emitted by `get-prover-tasks-summary`, plus a bounded
/// slice of matching entries for inspection.
#[derive(Serialize)]
pub(crate) struct ProverTasksSummaryInfo {
    pub(crate) total: usize,
    pub(crate) pending: usize,
    pub(crate) proving: usize,
    pub(crate) completed: usize,
    pub(crate) transient_failure: usize,
    pub(crate) permanent_failure: usize,
    pub(crate) matched: usize,
    pub(crate) returned: usize,
    pub(crate) entries: Vec<ProverTaskInfo>,
}

impl Formattable for ProverTasksSummaryInfo {
    fn format_porcelain(&self) -> String {
        let mut out = vec![
            porcelain_field("total", self.total),
            porcelain_field("pending", self.pending),
            porcelain_field("proving", self.proving),
            porcelain_field("completed", self.completed),
            porcelain_field("transient_failure", self.transient_failure),
            porcelain_field("permanent_failure", self.permanent_failure),
            porcelain_field("matched", self.matched),
            porcelain_field("returned", self.returned),
        ];
        for (i, entry) in self.entries.iter().enumerate() {
            out.push(porcelain_field(
                &format!("entries[{i}].key_hex"),
                &entry.key_hex,
            ));
            out.push(porcelain_field(
                &format!("entries[{i}].status"),
                entry.status.name,
            ));
            if let Some(rc) = entry.status.retry_count {
                out.push(porcelain_field(
                    &format!("entries[{i}].status.retry_count"),
                    rc,
                ));
            }
            out.push(porcelain_field(
                &format!("entries[{i}].updated_at_secs"),
                entry.updated_at_secs,
            ));
        }
        out.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_info_carries_retry_count_and_error_where_relevant() {
        let pending = StatusInfo::from(&TaskStatus::Pending);
        assert_eq!(pending.name, "pending");
        assert_eq!(pending.retry_count, None);
        assert_eq!(pending.error, None);

        let proving = StatusInfo::from(&TaskStatus::Proving { retry_count: 3 });
        assert_eq!(proving.name, "proving");
        assert_eq!(proving.retry_count, Some(3));
        assert_eq!(proving.error, None);

        let completed = StatusInfo::from(&TaskStatus::Completed);
        assert_eq!(completed.name, "completed");
        assert_eq!(completed.retry_count, None);
        assert_eq!(completed.error, None);

        let transient = StatusInfo::from(&TaskStatus::TransientFailure {
            retry_count: 2,
            error: "rpc down".into(),
        });
        assert_eq!(transient.name, "transient_failure");
        assert_eq!(transient.retry_count, Some(2));
        assert_eq!(transient.error.as_deref(), Some("rpc down"));

        let permanent = StatusInfo::from(&TaskStatus::PermanentFailure {
            error: "abandoned via dbtool".into(),
        });
        assert_eq!(permanent.name, "permanent_failure");
        assert_eq!(permanent.retry_count, None);
        assert_eq!(permanent.error.as_deref(), Some("abandoned via dbtool"));
    }

    #[test]
    fn prover_task_info_encodes_key_and_metadata_preview() {
        let key = vec![0xde, 0xad, 0xbe, 0xef];
        let mut record = TaskRecordData::new(TaskStatus::Pending);
        record.set_metadata(Some(vec![1, 2, 3]));

        let info = ProverTaskInfo::from_record(&key, &record);
        assert_eq!(info.key_hex, "deadbeef");
        assert_eq!(info.status.name, "pending");
        assert_eq!(info.metadata_len, 3);
        assert_eq!(info.metadata_hex.as_deref(), Some("010203"));
        assert_eq!(info.retry_after_secs, None);
    }

    #[test]
    fn prover_task_info_omits_metadata_when_absent() {
        let key = vec![0xaa];
        let record = TaskRecordData::new(TaskStatus::Completed);

        let info = ProverTaskInfo::from_record(&key, &record);
        assert_eq!(info.metadata_len, 0);
        assert_eq!(info.metadata_hex, None);
    }

    #[test]
    fn porcelain_output_includes_known_keys() {
        let key = vec![0xaa, 0xbb];
        let record = TaskRecordData::new(TaskStatus::Proving { retry_count: 1 });
        let info = ProverTaskInfo::from_record(&key, &record);

        let out = info.format_porcelain();
        assert!(out.contains("key_hex: aabb"));
        assert!(out.contains("status: proving"));
        assert!(out.contains("status.retry_count: 1"));
    }
}
