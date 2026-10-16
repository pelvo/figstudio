use crate::DiffReport;

impl DiffReport {
    /// Serializes the report in stable field/key order as compact natural JSON.
    ///
    /// Enum values become strings and byte buffers become arrays. Nonfinite
    /// numbers use a `$float_bits` object preserving their IEEE-754 bits.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}
