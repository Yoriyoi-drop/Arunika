//! Model threading untuk fase 3 MVP.
//!
//! Crate ini menyediakan model domain threading Android (Thread, ThreadState,
//! ThreadManager, dan Mutex model) tanpa thread OS nyata. Thread OS sebenarnya
//! memerlukan model keamanan dan pengujian yang sesuai (peringatan DESAIN.md:
//! "Jangan memperkenalkan JIT, loader ELF, atau akses host luas sebelum model
//! keamanan dan pengujian yang sesuai tersedia").
//!
//! Semua operasi di sini adalah model dalam proses dan dapat diuji secara
//! terisolasi.

use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

/// Kesalahan manajer thread.
#[derive(Debug, Error)]
pub enum ThreadError {
    #[error("thread tidak ditemukan: {0}")]
    NotFound(u64),
    #[error("tidak dapat memulai thread dalam status {0}")]
    InvalidStartState(String),
    #[error("tidak dapat menghentikan thread dalam status {0}")]
    InvalidStopState(String),
    #[error("id thread duplikat: {0}")]
    DuplicateId(u64),
}

/// Status siklus hidup thread Android.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ThreadState {
    New,
    Runnable,
    Running,
    Blocked,
    Waiting,
    TimedWaiting,
    Terminated,
}

impl ThreadState {
    /// Apakah transisi dari `dari` ke `ke` diperbolehkan.
    pub fn boleh_transisi(dari: ThreadState, ke: ThreadState) -> bool {
        use ThreadState::*;
        match (dari, ke) {
            // Tidak ada transisi keluar dari Terminated.
            (Terminated, _) => false,
            // New hanya bisa ke Runnable.
            (New, Runnable) => true,
            // Runnable bisa ke Running atau kembali New (belum dimulai).
            (Runnable, Running) | (Runnable, New) => true,
            // Running bisa ke Blocked, Waiting, TimedWaiting, atau Terminated.
            (Running, Blocked)
            | (Running, Waiting)
            | (Running, TimedWaiting)
            | (Running, Terminated) => true,
            // Blocked/Waiting/TimedWaiting bisa kembali ke Runnable.
            (Blocked, Runnable) | (Waiting, Runnable) | (TimedWaiting, Runnable) => true,
            _ => false,
        }
    }
}

impl std::fmt::Display for ThreadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThreadState::New => write!(f, "new"),
            ThreadState::Runnable => write!(f, "runnable"),
            ThreadState::Running => write!(f, "running"),
            ThreadState::Blocked => write!(f, "blocked"),
            ThreadState::Waiting => write!(f, "waiting"),
            ThreadState::TimedWaiting => write!(f, "timed-waiting"),
            ThreadState::Terminated => write!(f, "terminated"),
        }
    }
}

/// Satu thread dalam model domain.
#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub id: u64,
    pub nama: String,
    pub state: ThreadState,
    /// Apakah thread ini adalah daemon.
    pub daemon: bool,
    /// Prioritas thread (1-10, mirror Thread.MIN_PRIORITY..MAX_PRIORITY).
    pub prioritas: i32,
}

impl Thread {
    pub fn baru(id: u64, nama: impl Into<String>) -> Self {
        Self {
            id,
            nama: nama.into(),
            state: ThreadState::New,
            daemon: false,
            prioritas: 5,
        }
    }

    pub fn apakah_aktif(&self) -> bool {
        matches!(
            self.state,
            ThreadState::Runnable
                | ThreadState::Running
                | ThreadState::Blocked
                | ThreadState::Waiting
                | ThreadState::TimedWaiting
        )
    }
}

/// Manajer thread dalam proses.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ThreadManager {
    threads: HashMap<u64, Thread>,
    next_id: u64,
}

impl ThreadManager {
    pub fn baru() -> Self {
        Self::default()
    }

    /// Buat thread baru dalam status New.
    pub fn buat_thread(&mut self, nama: impl Into<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.threads.insert(id, Thread::baru(id, nama));
        id
    }

    /// Ambil referensi thread berdasarkan ID.
    pub fn thread(&self, id: u64) -> Option<&Thread> {
        self.threads.get(&id)
    }

    /// Mulai thread (New -> Runnable).
    pub fn mulai(&mut self, id: u64) -> Result<(), ThreadError> {
        let thread = self.threads.get_mut(&id).ok_or(ThreadError::NotFound(id))?;
        if !ThreadState::boleh_transisi(thread.state, ThreadState::Runnable) {
            return Err(ThreadError::InvalidStartState(thread.state.to_string()));
        }
        thread.state = ThreadState::Runnable;
        Ok(())
    }

    /// Jalankan thread (Runnable -> Running).
    pub fn jalankan(&mut self, id: u64) -> Result<(), ThreadError> {
        let thread = self.threads.get_mut(&id).ok_or(ThreadError::NotFound(id))?;
        if !ThreadState::boleh_transisi(thread.state, ThreadState::Running) {
            return Err(ThreadError::InvalidStartState(thread.state.to_string()));
        }
        thread.state = ThreadState::Running;
        Ok(())
    }

    /// Blokir thread (Running -> Blocked).
    pub fn blokir(&mut self, id: u64) -> Result<(), ThreadError> {
        let thread = self.threads.get_mut(&id).ok_or(ThreadError::NotFound(id))?;
        if !ThreadState::boleh_transisi(thread.state, ThreadState::Blocked) {
            return Err(ThreadError::InvalidStopState(thread.state.to_string()));
        }
        thread.state = ThreadState::Blocked;
        Ok(())
    }

    /// Akhiri thread (apapun status non-Terminated -> Terminated).
    pub fn akhiri(&mut self, id: u64) -> Result<(), ThreadError> {
        let thread = self.threads.get_mut(&id).ok_or(ThreadError::NotFound(id))?;
        if thread.state == ThreadState::Terminated {
            return Err(ThreadError::InvalidStopState("terminated".into()));
        }
        thread.state = ThreadState::Terminated;
        Ok(())
    }

    /// Jumlah thread.
    pub fn jumlah(&self) -> usize {
        self.threads.len()
    }

    pub fn apakah_kosong(&self) -> bool {
        self.threads.is_empty()
    }

    /// Semua ID thread, terurut.
    pub fn id_thread(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.threads.keys().copied().collect();
        ids.sort();
        ids
    }

    /// Hapus semua thread yang sudah Terminated.
    pub fn bersihkan_terminated(&mut self) -> usize {
        let sebelum = self.threads.len();
        self.threads
            .retain(|_, t| t.state != ThreadState::Terminated);
        sebelum - self.threads.len()
    }
}

/// Mutex model sederhana (tidak ada sinkronisasi OS nyata).
#[derive(Debug, Clone, Serialize)]
pub struct Mutex<T> {
    nilai: Option<T>,
    terkunci: bool,
}

impl<T> Mutex<T> {
    pub fn baru(nilai: T) -> Self {
        Self {
            nilai: Some(nilai),
            terkunci: false,
        }
    }

    pub fn kunci(&mut self) -> Result<(), ThreadError> {
        if self.terkunci {
            return Err(ThreadError::InvalidStartState("sudah terkunci".into()));
        }
        self.terkunci = true;
        Ok(())
    }

    pub fn lepas_kunci(&mut self) -> Result<(), ThreadError> {
        if !self.terkunci {
            return Err(ThreadError::InvalidStopState("tidak terkunci".into()));
        }
        self.terkunci = false;
        Ok(())
    }

    pub fn nilai(&self) -> Option<&T> {
        self.nilai.as_ref()
    }
}

impl<T> Default for Mutex<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::baru(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manajer_awal_kosong() {
        let m = ThreadManager::baru();
        assert!(m.apakah_kosong());
        assert_eq!(m.jumlah(), 0);
    }

    #[test]
    fn buat_thread_dapatkan_id_berurutan() {
        let mut m = ThreadManager::baru();
        let id1 = m.buat_thread("thread-1".to_string());
        let id2 = m.buat_thread("thread-2".to_string());
        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
        assert_eq!(m.jumlah(), 2);
    }

    #[test]
    fn siklus_status_thread() {
        let mut m = ThreadManager::baru();
        let id = m.buat_thread("worker".to_string());
        assert_eq!(m.thread(id).unwrap().state, ThreadState::New);
        assert!(!m.thread(id).unwrap().apakah_aktif());

        m.mulai(id).unwrap();
        assert_eq!(m.thread(id).unwrap().state, ThreadState::Runnable);
        assert!(m.thread(id).unwrap().apakah_aktif());

        m.jalankan(id).unwrap();
        assert_eq!(m.thread(id).unwrap().state, ThreadState::Running);

        m.blokir(id).unwrap();
        assert_eq!(m.thread(id).unwrap().state, ThreadState::Blocked);

        m.mulai(id).unwrap(); // Blocked -> Runnable
        assert_eq!(m.thread(id).unwrap().state, ThreadState::Runnable);

        m.akhiri(id).unwrap();
        assert_eq!(m.thread(id).unwrap().state, ThreadState::Terminated);
        assert!(!m.thread(id).unwrap().apakah_aktif());
    }

    #[test]
    fn transisi_tidak_valid_ditolak() {
        let mut m = ThreadManager::baru();
        let id = m.buat_thread("t".to_string());
        // New tidak bisa langsung Running.
        let err = m.jalankan(id).unwrap_err();
        assert!(matches!(err, ThreadError::InvalidStartState { .. }));

        m.mulai(id).unwrap();
        // Running tidak bisa langsung New.
        let err = m.mulai(id).unwrap_err();
        assert!(matches!(err, ThreadError::InvalidStartState { .. }));

        m.akhiri(id).unwrap();
        // Terminated tidak bisa diakhiri lagi.
        let err = m.akhiri(id).unwrap_err();
        assert!(matches!(err, ThreadError::InvalidStopState { .. }));
    }

    #[test]
    fn thread_tidak_ditemukan() {
        let mut m = ThreadManager::baru();
        assert!(m.mulai(99).is_err());
        assert!(m.thread(99).is_none());
    }

    #[test]
    fn id_thread_terurut() {
        let mut m = ThreadManager::baru();
        m.buat_thread("c".to_string());
        m.buat_thread("a".to_string());
        m.buat_thread("b".to_string());
        assert_eq!(m.id_thread(), vec![0, 1, 2]);
    }

    #[test]
    fn bersihkan_terminated() {
        let mut m = ThreadManager::baru();
        let id1 = m.buat_thread("a".to_string());
        let id2 = m.buat_thread("b".to_string());
        m.akhiri(id1).unwrap();
        assert_eq!(m.jumlah(), 2);
        let dihapus = m.bersihkan_terminated();
        assert_eq!(dihapus, 1);
        assert_eq!(m.jumlah(), 1);
        assert!(m.thread(id1).is_none());
        assert!(m.thread(id2).is_some());
    }

    #[test]
    fn mutex_kunci_dan_lepas() {
        let mut mx = Mutex::baru(42i32);
        mx.kunci().unwrap();
        assert_eq!(*mx.nilai().unwrap(), 42);
        let err = mx.kunci().unwrap_err();
        assert!(matches!(err, ThreadError::InvalidStartState { .. }));
        mx.lepas_kunci().unwrap();
        mx.lepas_kunci().unwrap_err(); // sudah tidak terkunci
    }

    #[test]
    fn thread_serialisasi_json() {
        let t = Thread::baru(1, "main".to_string());
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("main"));
        assert!(json.contains("\"id\":1"));
    }

    #[test]
    fn thread_error_tampilkan() {
        let e = ThreadError::NotFound(5);
        assert_eq!(e.to_string(), "thread tidak ditemukan: 5");
        let e2 = ThreadError::DuplicateId(7);
        assert_eq!(e2.to_string(), "id thread duplikat: 7");
    }

    #[test]
    fn thread_state_boleh_transisi() {
        assert!(ThreadState::boleh_transisi(
            ThreadState::New,
            ThreadState::Runnable
        ));
        assert!(!ThreadState::boleh_transisi(
            ThreadState::New,
            ThreadState::Running
        ));
        assert!(!ThreadState::boleh_transisi(
            ThreadState::Terminated,
            ThreadState::Runnable
        ));
    }
}
