//! Uji integrasi publik untuk `arunika-activity-runtime` (fase 2 MVP).
//!
//! Memverifikasi model lifecycle Activity, intent eksplisit/implisit, dan
//! tumpukan task dari luar crate.

use arunika_activity_runtime::{
    ActivityError, ActivityRecord, ActivityState, ComponentName, Intent, TaskStack,
};

#[test]
fn aktivitas_baru_dimulai_di_status_created() {
    let comp = ComponentName::new("com.contoh", "MainActivity");
    let a = ActivityRecord::new(1, comp.clone(), "task-1");
    assert_eq!(a.state, ActivityState::Created);
    assert_eq!(a.component, comp);
    assert_eq!(a.task_id, "task-1");
}

#[test]
fn transisi_lifecycle_normal_boleh() {
    let comp = ComponentName::new("com.contoh", "MainActivity");
    let mut a = ActivityRecord::new(1, comp, "task-1");
    a.transition_to(ActivityState::Started).unwrap();
    assert_eq!(a.state, ActivityState::Started);
    a.transition_to(ActivityState::Resumed).unwrap();
    assert_eq!(a.state, ActivityState::Resumed);
    a.transition_to(ActivityState::Paused).unwrap();
    assert_eq!(a.state, ActivityState::Paused);
    a.transition_to(ActivityState::Stopped).unwrap();
    assert_eq!(a.state, ActivityState::Stopped);
    a.transition_to(ActivityState::Destroyed).unwrap();
    assert_eq!(a.state, ActivityState::Destroyed);
}

#[test]
fn transisi_lifecycle_tidak_valid_ditolak() {
    let comp = ComponentName::new("com.contoh", "MainActivity");
    let mut a = ActivityRecord::new(1, comp, "task-1");
    // Created tidak bisa langsung ke Destroyed.
    let err = a.transition_to(ActivityState::Destroyed).unwrap_err();
    assert!(matches!(err, ActivityError::InvalidTransition { .. }));
    // Setelah Destroyed tidak ada transisi keluar.
    a.transition_to(ActivityState::Started).unwrap();
    a.transition_to(ActivityState::Resumed).unwrap();
    a.transition_to(ActivityState::Paused).unwrap();
    a.transition_to(ActivityState::Stopped).unwrap();
    a.transition_to(ActivityState::Destroyed).unwrap();
    let err = a.transition_to(ActivityState::Resumed).unwrap_err();
    assert!(matches!(err, ActivityError::InvalidTransition { .. }));
}

#[test]
fn intent_eksplisit_dan_implisit() {
    let comp = ComponentName::new("com.contoh", "TargetActivity");
    let explisit = Intent::explicit(comp.clone());
    assert!(explisit.is_explicit());
    assert_eq!(explisit.component.unwrap(), comp);

    let implisit = Intent::with_action("android.intent.action.MAIN");
    assert!(!implisit.is_explicit());
    assert_eq!(
        implisit.action,
        Some("android.intent.action.MAIN".to_string())
    );
}

#[test]
fn intent_put_extra_menyimpan_pasangan_kunci_nilai() {
    let comp = ComponentName::new("com.contoh", "MainActivity");
    let intent = Intent::explicit(comp)
        .put_extra("kunci1", "nilai1")
        .put_extra("kunci2", "nilai2");
    assert_eq!(intent.extras.get("kunci1"), Some(&"nilai1".to_string()));
    assert_eq!(intent.extras.get("kunci2"), Some(&"nilai2".to_string()));
}

#[test]
fn task_stack_mulai_dan_akhiri_aktivitas() {
    let mut stack = TaskStack::new();
    let comp = ComponentName::new("com.contoh", "MainActivity");
    let intent = Intent::new();
    let id = stack.start_activity(comp.clone(), intent, "task-1");
    assert_eq!(id, 0);
    assert_eq!(stack.len(), 1);
    assert_eq!(stack.top().unwrap().id, id);
    assert_eq!(stack.top().unwrap().state, ActivityState::Created);

    // Transisi lifecycle diuji secara terpadu di unit test ActivityRecord.
    let _ = stack.finish_top().unwrap();
    assert!(stack.is_empty());
}

#[test]
fn task_stack_find_activity_berdasarkan_id() {
    let mut stack = TaskStack::new();
    let comp1 = ComponentName::new("com.contoh", "A");
    let comp2 = ComponentName::new("com.contoh", "B");
    let intent = Intent::new();
    stack.start_activity(comp1, intent.clone(), "task-1");
    stack.start_activity(comp2.clone(), intent, "task-1");

    let ditemukan = stack.find(1).unwrap();
    assert_eq!(ditemukan.component, comp2);
    let tidak_ditemukan = stack.find(99);
    assert!(tidak_ditemukan.is_none());
}

#[test]
fn activity_error_tampilkan_pesan_jelas() {
    let err = ActivityError::InvalidTransition {
        from: "created".to_string(),
        to: "destroyed".to_string(),
    };
    assert!(err.to_string().contains("created"));
    assert!(err.to_string().contains("destroyed"));
    let err = ActivityError::ActivityNotFound("MainActivity".to_string());
    assert!(err.to_string().contains("MainActivity"));
}
