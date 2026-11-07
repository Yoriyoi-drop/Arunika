use serde::Serialize;
use thiserror::Error;

mod native;

pub use native::{AndroidNativeMethods, AppManifest};

#[derive(Debug, Error)]
pub enum FrameworkError {
    #[error("API Android belum tersedia: {0}")]
    Unavailable(String),
    #[error("runtime belum siap: {0}")]
    NotImplemented(String),
    #[error("konteks runtime tidak valid: {0}")]
    InvalidContext(String),
}

/// Model selidik tahap awal untuk cakupan framework API.
///
/// Kelompok API mengikuti pendekatan bertahap di DESIGN.md.
/// Saat ini masih dalam tahap definisi, bukan implementasi.
#[derive(Debug, Clone, Serialize)]
pub struct ApiCoverageGroup {
    pub group: String,
    pub available: Vec<String>,
    pub unavailable: Vec<String>,
    pub phase: String,
}

impl ApiCoverageGroup {
    pub fn new(group: &str, phase: &str) -> Self {
        Self {
            group: group.to_owned(),
            available: Vec::new(),
            unavailable: Vec::new(),
            phase: phase.to_owned(),
        }
    }

    pub fn declare_available(&mut self, api: &str) {
        if !self.available.contains(&api.to_owned()) {
            self.available.push(api.to_owned());
        }
    }

    pub fn declare_unavailable(&mut self, api: &str) {
        if !self.unavailable.contains(&api.to_owned()) {
            self.unavailable.push(api.to_owned());
        }
    }
}

impl Default for ApiCoverageGroup {
    fn default() -> Self {
        Self::new("android.util", "fase-0")
    }
}

/// Model kesadaran lintas kelompok API yang sedang dibangun.
#[derive(Debug, Clone, Serialize)]
pub struct FrameworkCoverageModel {
    pub groups: Vec<ApiCoverageGroup>,
}

impl FrameworkCoverageModel {
    pub fn empty() -> Self {
        Self { groups: Vec::new() }
    }

    pub fn with_group(group: &str, phase: &str) -> Self {
        Self {
            groups: vec![ApiCoverageGroup::new(group, phase)],
        }
    }

    pub fn declare_available(&mut self, group_name: &str, api: &str) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.group == group_name) {
            group.declare_available(api);
        } else {
            let mut group = ApiCoverageGroup::new(group_name, "fase-0");
            group.declare_available(api);
            self.groups.push(group);
        }
    }

    pub fn declare_unavailable(&mut self, group_name: &str, api: &str) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.group == group_name) {
            group.declare_unavailable(api);
        } else {
            let mut group = ApiCoverageGroup::new(group_name, "fase-0");
            group.declare_unavailable(api);
            self.groups.push(group);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiCoverageGroup, FrameworkCoverageModel};

    #[test]
    fn group_default_milik_kelompok_awal() {
        let g = ApiCoverageGroup::default();
        assert_eq!(g.group, "android.util");
        assert_eq!(g.phase, "fase-0");
    }

    #[test]
    fn available_ditambahkan_ke_group() {
        let mut g = ApiCoverageGroup::new("android.os", "fase-0");
        g.declare_available("Bundle");
        assert!(g.available.contains(&"Bundle".to_owned()));
    }

    #[test]
    fn unavailable_ditambahkan_ke_group() {
        let mut g = ApiCoverageGroup::new("android.app", "fase-0");
        g.declare_unavailable("Activity");
        assert!(g.unavailable.contains(&"Activity".to_owned()));
    }

    #[test]
    fn model_sederhana_menyimpan_satu_group() {
        let m = FrameworkCoverageModel::with_group("android.os", "fase-0");
        assert_eq!(m.groups.len(), 1);
        assert_eq!(m.groups[0].group, "android.os");
    }

    #[test]
    fn model_menambahkan_available_dengan_group_baru() {
        let mut m = FrameworkCoverageModel::empty();
        m.declare_available("android.os", "Bundle");
        assert!(m.groups.iter().any(|g| g.group == "android.os"));
        assert!(
            m.groups
                .iter()
                .any(|g| g.available.contains(&"Bundle".to_owned()))
        );
    }

    #[test]
    fn model_menambahkan_unavailable_dengan_group_baru() {
        let mut m = FrameworkCoverageModel::empty();
        m.declare_unavailable("android.app", "Activity");
        assert!(m.groups.iter().any(|g| g.group == "android.app"));
        assert!(
            m.groups
                .iter()
                .any(|g| g.unavailable.contains(&"Activity".to_owned()))
        );
    }
}
