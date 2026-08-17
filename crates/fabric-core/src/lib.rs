//! Core domain library for the Personal Knowledge Fabric.

/// Returns a diagnostic string used by the initial CLI smoke test.
#[must_use]
pub const fn status() -> &'static str {
    "Personal Knowledge Fabric bootstrap is ready."
}

#[cfg(test)]
mod tests {
    use super::status;

    #[test]
    fn status_identifies_the_project() {
        assert!(status().contains("Knowledge Fabric"));
    }
}
