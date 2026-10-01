//! Measurements shared by transaction admission and checkpoint size checks.

use strata_ol_chain_types_v1::OLLog;

/// Accumulates log count and encoded sizes without applying budget limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogUsage {
    count: usize,
    payload_bytes: usize,
}

impl LogUsage {
    /// Measures the payloads of already encoded [`OLLog`] entries.
    pub fn from_logs(logs: &[OLLog]) -> Self {
        let mut usage = Self::default();
        usage.add_logs(logs);
        usage
    }

    /// Adds existing log entries to the accumulated measurements.
    pub fn add_logs(&mut self, logs: &[OLLog]) {
        for log in logs {
            self.add_payload(log.payload());
        }
    }

    /// Adds one complete encoded log payload, including its type prefix.
    ///
    /// Excludes outer container fields, which [`Self::ssz_size`] accounts for.
    pub fn add_payload(&mut self, payload: &[u8]) {
        self.count += 1;
        self.payload_bytes += payload.len();
    }

    /// Returns the accumulated number of logs.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Returns total payload bytes, excluding SSZ list and container framing.
    pub fn payload_bytes(&self) -> usize {
        self.payload_bytes
    }

    /// Returns the encoded size of an SSZ list containing the accumulated logs.
    pub fn ssz_size(&self) -> usize {
        // Each log adds a list offset (4), account serial (4), and payload offset (4).
        self.payload_bytes + 12 * self.count
    }
}

#[cfg(test)]
mod tests {
    use ssz::Encode;
    use strata_identifiers::AccountSerial;
    use strata_ol_chain_types_v1::MAX_LOG_PAYLOAD_LEN;

    use super::*;

    #[test]
    fn test_log_usage_matches_ssz_encoding() {
        let mut logs = Vec::new();
        let empty = LogUsage::from_logs(&logs);
        assert_eq!(empty.count(), 0);
        assert_eq!(empty.payload_bytes(), 0);
        assert_eq!(empty.ssz_size(), logs.as_ssz_bytes().len());

        for payload_len in [0, 21, 127, 128, MAX_LOG_PAYLOAD_LEN as usize] {
            logs.push(OLLog::new(AccountSerial::one(), vec![0x42; payload_len]));
            let usage = LogUsage::from_logs(&logs);
            assert_eq!(usage.count(), logs.len());
            assert_eq!(
                usage.payload_bytes(),
                logs.iter().map(|log| log.payload().len()).sum::<usize>()
            );
            assert_eq!(usage.ssz_size(), logs.as_ssz_bytes().len());
        }
    }

    #[test]
    fn test_incremental_log_usage_matches_ssz_encoding() {
        let logs = vec![
            OLLog::new(AccountSerial::one(), vec![0x42; 21]),
            OLLog::new(AccountSerial::one(), vec![0x43; 81]),
            OLLog::new(AccountSerial::one(), vec![]),
        ];
        let mut usage = LogUsage::from_logs(&logs[..1]);
        usage.add_logs(&logs[1..2]);
        usage.add_payload(logs[2].payload());

        assert_eq!(usage, LogUsage::from_logs(&logs));
        assert_eq!(usage.ssz_size(), logs.as_ssz_bytes().len());
    }
}
