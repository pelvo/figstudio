use crate::DiffReport;

impl DiffReport {
    /// Serializes the report in stable field/key order as compact natural JSON.
    ///
    /// Enum values become strings and byte buffers become arrays. Nonfinite
    /// numbers return an error rather than silently becoming JSON null.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}
