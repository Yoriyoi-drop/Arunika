use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ActivityError {
    #[error("lifecycle belum didukung: {0}")]
    LifecycleUnavailable(String),
    #[error("intent tidak dapat diselesaikan: {0}")]
    IntentUnresolved(String),
    #[error("konteks aktivitas tidak tersedia: {0}")]
    ContextUnavailable(String),
    #[error("transisi lifecycle tidak valid: {from} -> {to}")]
    InvalidTransition { from: String, to: String },
    #[error("aktivitas tidak ditemukan dalam task: {0}")]
    ActivityNotFound(String),
}

/// Status lifecycle Activity Android.
///
/// Urutan normal: Created → Started → Resumed → Paused → Stopped → Destroyed.
/// Setiap transisi memvalidasi bahwa status tujuan dapat tercapai dari status
/// saat ini, sehingga aktivitas tidak pernah melompat ke status yang tidak
/// sesuai (mis. Created langsung ke Destroyed tanpa melalui Started/Resumed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityState {
    Created,
    Started,
    Resumed,
    Paused,
    Stopped,
    Destroyed,
}

impl ActivityState {
    /// Apakah transisi dari `from` ke `to` diperbolehkan menurut lifecycle Android.
    pub fn may_transition(from: ActivityState, to: ActivityState) -> bool {
        use ActivityState::*;
        match (from, to) {
            // Tidak ada transisi keluar dari Destroyed.
            (Destroyed, _) => false,
            // Created hanya dapat melanjutkan ke Started.
            (Created, Started) => true,
            // Started dapat kembali ke Created atau melanjutkan ke Resumed.
            (Started, Created) | (Started, Resumed) => true,
            // Resumed dapat turun ke Paused atau kembali ke Started.
            (Resumed, Started) | (Resumed, Paused) => true,
            // Paused dapat kembali ke Resumed atau turun ke Stopped.
            (Paused, Resumed) | (Paused, Stopped) => true,
            // Stopped dapat kembali ke Paused atau ke Destroyed.
            (Stopped, Paused) | (Stopped, Destroyed) => true,
            // Status sama tidak diperlukan transisi.
            _ => false,
        }
    }
}

impl std::fmt::Display for ActivityState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivityState::Created => write!(f, "created"),
            ActivityState::Started => write!(f, "started"),
            ActivityState::Resumed => write!(f, "resumed"),
            ActivityState::Paused => write!(f, "paused"),
            ActivityState::Stopped => write!(f, "stopped"),
            ActivityState::Destroyed => write!(f, "destroyed"),
        }
    }
}

/// Deskripsi komponen Activity (paket + nama kelas).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentName {
    pub package: String,
    pub class: String,
}

impl ComponentName {
    pub fn new(package: impl Into<String>, class: impl Into<String>) -> Self {
        Self {
            package: package.into(),
            class: class.into(),
        }
    }

    /// Descriptor DEX dari komponen ini, misalnya `Lcom/contoh/MainActivity;`.
    pub fn to_descriptor(&self) -> String {
        format!("L{}/{};", self.package.replace('.', "/"), self.class)
    }
}

/// Intent Android: permintaan untuk melakukan tindakan, biasanya memulai Activity.
///
/// Intent bersifat implisit (hanya action) atau eksplisit (action + component).
/// Extras disimpan sebagai pasangan kunci/nilai string untuk MVP.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Intent {
    /// Tindakan yang diminta, misalnya `android.intent.action.MAIN`.
    pub action: Option<String>,
    /// Komponen target bila intent bersifat eksplisit.
    pub component: Option<ComponentName>,
    /// Data URI yang terkait dengan intent.
    pub data: Option<String>,
    /// Ekstra parameter tambahan.
    pub extras: std::collections::BTreeMap<String, String>,
}

impl Intent {
    pub fn new() -> Self {
        Self::default()
    }

    /// Buat Intent eksplisit yang menargetkan komponen tertentu.
    pub fn explicit(component: ComponentName) -> Self {
        Self {
            component: Some(component),
            ..Self::default()
        }
    }

    /// Buat Intent implisit dengan action tertentu.
    pub fn with_action(action: impl Into<String>) -> Self {
        Self {
            action: Some(action.into()),
            ..Self::default()
        }
    }

    /// Apakah intent ini menargetkan komponen tertentu (eksplisit).
    pub fn is_explicit(&self) -> bool {
        self.component.is_some()
    }

    /// Tambahkan pasangan kunci/nilai ke extras.
    pub fn put_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extras.insert(key.into(), value.into());
        self
    }
}

/// Satu aktivitas yang sedang dijalankan dalam sebuah task.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityRecord {
    /// Identifikasi unik aktivitas dalam proses.
    pub id: u64,
    /// Komponen yang mengimplementasikan aktivitas ini.
    pub component: ComponentName,
    /// Status lifecycle saat ini.
    pub state: ActivityState,
    /// Intent yang digunakan untuk memulai aktivitas.
    pub intent: Option<Intent>,
    /// Task yang merupakan milik aktivitas ini.
    pub task_id: String,
}

impl ActivityRecord {
    pub fn new(id: u64, component: ComponentName, task_id: impl Into<String>) -> Self {
        Self {
            id,
            component,
            state: ActivityState::Created,
            intent: None,
            task_id: task_id.into(),
        }
    }

    /// Pindah aktivitas ke status baru, atau gagal bila transisi tidak valid.
    pub fn transition_to(&mut self, to: ActivityState) -> Result<(), ActivityError> {
        if !ActivityState::may_transition(self.state, to) {
            return Err(ActivityError::InvalidTransition {
                from: self.state.to_string(),
                to: to.to_string(),
            });
        }
        self.state = to;
        Ok(())
    }

    /// Pasang intent ke aktivitas.
    pub fn with_intent(mut self, intent: Intent) -> Self {
        self.intent = Some(intent);
        self
    }
}

/// Stack task yang berisi daftar aktivitas.
///
/// Aktivitas teratas adalah yang sedang berjalan dan menerima input/fokus.
#[derive(Debug, Clone, Serialize, Default)]
pub struct TaskStack {
    activities: Vec<ActivityRecord>,
    next_id: u64,
}

impl TaskStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Jumlah aktivitas dalam task.
    pub fn len(&self) -> usize {
        self.activities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.activities.is_empty()
    }

    /// Aktivitas teratas, bila ada.
    pub fn top(&self) -> Option<&ActivityRecord> {
        self.activities.last()
    }

    /// Mulai aktivitas baru dan dorongnya ke puncak task.
    pub fn start_activity(
        &mut self,
        component: ComponentName,
        intent: Intent,
        task_id: impl Into<String>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let record = ActivityRecord::new(id, component, task_id).with_intent(intent);
        self.activities.push(record);
        id
    }

    /// Akhiri aktivitas teratas dan keluarkannya dari task.
    pub fn finish_top(&mut self) -> Option<ActivityRecord> {
        self.activities.pop()
    }

    /// Cari aktivitas berdasarkan ID.
    pub fn find(&self, id: u64) -> Option<&ActivityRecord> {
        self.activities.iter().find(|a| a.id == id)
    }

    /// Semua aktivitas, dari bawah ke atas.
    pub fn activities(&self) -> &[ActivityRecord] {
        &self.activities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_state_transisi_normal_boleh() {
        assert!(ActivityState::may_transition(
            ActivityState::Created,
            ActivityState::Started
        ));
        assert!(ActivityState::may_transition(
            ActivityState::Started,
            ActivityState::Resumed
        ));
        assert!(ActivityState::may_transition(
            ActivityState::Resumed,
            ActivityState::Paused
        ));
        assert!(ActivityState::may_transition(
            ActivityState::Paused,
            ActivityState::Stopped
        ));
        assert!(ActivityState::may_transition(
            ActivityState::Stopped,
            ActivityState::Destroyed
        ));
    }

    #[test]
    fn activity_state_transisi_tidak_valid_ditolak() {
        // Created tidak boleh langsung ke Destroyed.
        assert!(!ActivityState::may_transition(
            ActivityState::Created,
            ActivityState::Destroyed
        ));
        // Destroyed tidak boleh bertransisi ke mana pun.
        assert!(!ActivityState::may_transition(
            ActivityState::Destroyed,
            ActivityState::Created
        ));
        // Resumed tidak boleh langsung ke Stopped.
        assert!(!ActivityState::may_transition(
            ActivityState::Resumed,
            ActivityState::Stopped
        ));
    }

    #[test]
    fn activity_record_transisi_dan_status() {
        let comp = ComponentName::new("com.contoh", "MainActivity");
        let mut a = ActivityRecord::new(1, comp.clone(), "task-1");
        assert_eq!(a.state, ActivityState::Created);
        assert_eq!(a.component, comp);
        assert_eq!(a.task_id, "task-1");

        a.transition_to(ActivityState::Started).unwrap();
        a.transition_to(ActivityState::Resumed).unwrap();
        assert_eq!(a.state, ActivityState::Resumed);

        let err = a.transition_to(ActivityState::Stopped).unwrap_err();
        assert!(matches!(err, ActivityError::InvalidTransition { .. }));
    }

    #[test]
    fn intent_eksplisit_dan_implisit() {
        let comp = ComponentName::new("com.contoh", "TargetActivity");
        let expl = Intent::explicit(comp.clone());
        assert!(expl.is_explicit());
        assert_eq!(expl.component.unwrap(), comp);

        let impl_ = Intent::with_action("android.intent.action.VIEW");
        assert!(!impl_.is_explicit());
        assert_eq!(impl_.action.as_deref(), Some("android.intent.action.VIEW"));

        let with_extra = Intent::with_action("act").put_extra("kunci", "nilai");
        assert_eq!(with_extra.extras.get("kunci"), Some(&"nilai".to_string()));
    }

    #[test]
    fn component_to_descriptor() {
        let comp = ComponentName::new("com.contoh.app", "MainActivity");
        assert_eq!(comp.to_descriptor(), "Lcom/contoh/app/MainActivity;");
    }

    #[test]
    fn task_stack_mulai_dan_akhiri_aktivitas() {
        let mut stack = TaskStack::new();
        assert!(stack.is_empty());

        let comp = ComponentName::new("com.contoh", "MainActivity");
        let id = stack.start_activity(
            comp,
            Intent::with_action("android.intent.action.MAIN"),
            "task-1",
        );
        assert_eq!(id, 0);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.top().unwrap().id, 0);

        let id2 = stack.start_activity(
            ComponentName::new("com.contoh", "Second"),
            Intent::new(),
            "task-1",
        );
        assert_eq!(id2, 1);
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.top().unwrap().id, 1);

        let selesai = stack.finish_top().unwrap();
        assert_eq!(selesai.id, 1);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.top().unwrap().id, 0);
    }

    #[test]
    fn task_stack_find_dan_activities() {
        let mut stack = TaskStack::new();
        stack.start_activity(ComponentName::new("com.contoh", "A"), Intent::new(), "t");
        stack.start_activity(ComponentName::new("com.contoh", "B"), Intent::new(), "t");
        assert!(stack.find(0).is_some());
        assert!(stack.find(99).is_none());
        assert_eq!(stack.activities().len(), 2);
    }

    #[test]
    fn activity_error_tampilkan() {
        let e = ActivityError::InvalidTransition {
            from: "created".into(),
            to: "destroyed".into(),
        };
        assert_eq!(
            e.to_string(),
            "transisi lifecycle tidak valid: created -> destroyed"
        );
        let e2 = ActivityError::ActivityNotFound("id-5".into());
        assert_eq!(e2.to_string(), "aktivitas tidak ditemukan dalam task: id-5");
    }

    #[test]
    fn activity_record_serialisasi_json() {
        let comp = ComponentName::new("com.contoh", "MainActivity");
        let a = ActivityRecord::new(1, comp, "task-1");
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("com.contoh"));
        assert!(json.contains("MainActivity"));
        assert!(json.contains("\"state\":\"created\""));
    }

    #[test]
    fn intent_default_kosong() {
        let i = Intent::new();
        assert!(i.action.is_none());
        assert!(i.component.is_none());
        assert!(i.extras.is_empty());
        assert!(!i.is_explicit());
    }
}
