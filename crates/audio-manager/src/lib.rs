//! Manajer audio untuk fase 2 MVP.
//!
//! Crate ini menyediakan model domain audio Android (AudioManager, AudioTrack,
//! status pemutaran, volume) tanpa integrasi platform host. Integrasi audio
//! nyata (PipeWire di Linux, CoreAudio di macOS) dilakukan melalui
//! `host-linux`/`host-macos` dan belum tersedia pada Fase 2.
//!
//! Semua operasi di sini bersifat memori dalam proses dan dapat diuji secara
//! terisolasi.

use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

/// Kesalahan manajer audio.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("track tidak ditemukan: {0}")]
    NotFound(u64),
    #[error("track sudah diputar: {0}")]
    AlreadyPlaying(u64),
    #[error("tidak dapat memutar track dalam status {0}")]
    InvalidState(String),
    #[error("volume di luar rentang 0.0..=1.0: {0}")]
    InvalidVolume(f32),
}

/// Status pemutaran audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AudioState {
    Idle,
    Playing,
    Paused,
    Stopped,
}

impl std::fmt::Display for AudioState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioState::Idle => write!(f, "idle"),
            AudioState::Playing => write!(f, "playing"),
            AudioState::Paused => write!(f, "paused"),
            AudioState::Stopped => write!(f, "stopped"),
        }
    }
}

/// Satu track audio.
#[derive(Debug, Clone, Serialize)]
pub struct AudioTrack {
    /// ID track (berurutan).
    pub id: u64,
    /// Path atau URI sumber audio.
    pub source: String,
    /// Status pemutaran saat ini.
    pub state: AudioState,
    /// Volume 0.0..=1.0.
    pub volume: f32,
    /// Apakah track diputar berulang.
    pub looping: bool,
    /// Posisi pemutaran saat ini (detik, estimasi).
    pub position_secs: f64,
}

impl AudioTrack {
    pub fn new(id: u64, source: impl Into<String>) -> Self {
        Self {
            id,
            source: source.into(),
            state: AudioState::Idle,
            volume: 1.0,
            looping: false,
            position_secs: 0.0,
        }
    }

    /// Apakah track sedang diputar (Playing atau Paused).
    pub fn is_active(&self) -> bool {
        matches!(self.state, AudioState::Playing | AudioState::Paused)
    }
}

/// Manajer audio dalam proses.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AudioManager {
    tracks: HashMap<u64, AudioTrack>,
    next_id: u64,
    /// Volume global utama 0.0..=1.0.
    master_volume: f32,
}

const MAX_VOLUME: f32 = 1.0;
const MIN_VOLUME: f32 = 0.0;

impl AudioManager {
    pub fn new() -> Self {
        Self {
            master_volume: 1.0,
            ..Self::default()
        }
    }

    /// Muat sumber audio dan buat track baru. Kembalikan ID track.
    pub fn load(&mut self, source: impl Into<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let track = AudioTrack::new(id, source);
        self.tracks.insert(id, track);
        id
    }

    /// Ambil referensi track berdasarkan ID.
    pub fn track(&self, id: u64) -> Option<&AudioTrack> {
        self.tracks.get(&id)
    }

    /// Mulai memutar track. Track yang sudah Playing ditolak.
    pub fn play(&mut self, id: u64) -> Result<(), AudioError> {
        let track = self.tracks.get_mut(&id).ok_or(AudioError::NotFound(id))?;
        if track.state == AudioState::Playing {
            return Err(AudioError::AlreadyPlaying(id));
        }
        track.state = AudioState::Playing;
        track.position_secs = 0.0;
        Ok(())
    }

    /// Jeda track yang sedang Playing.
    pub fn pause(&mut self, id: u64) -> Result<(), AudioError> {
        let track = self.tracks.get_mut(&id).ok_or(AudioError::NotFound(id))?;
        if track.state != AudioState::Playing {
            return Err(AudioError::InvalidState(format!(
                "tidak dapat jeda track dalam status {}",
                track.state
            )));
        }
        track.state = AudioState::Paused;
        Ok(())
    }

    /// Hentikan pemutaran track.
    pub fn stop(&mut self, id: u64) -> Result<(), AudioError> {
        let track = self.tracks.get_mut(&id).ok_or(AudioError::NotFound(id))?;
        track.state = AudioState::Stopped;
        track.position_secs = 0.0;
        Ok(())
    }

    /// Atur volume track (0.0..=1.0).
    pub fn set_volume(&mut self, id: u64, volume: f32) -> Result<(), AudioError> {
        if volume < MIN_VOLUME || volume > MAX_VOLUME {
            return Err(AudioError::InvalidVolume(volume));
        }
        let track = self.tracks.get_mut(&id).ok_or(AudioError::NotFound(id))?;
        track.volume = volume;
        Ok(())
    }

    /// Atur volume global utama (0.0..=1.0).
    pub fn set_master_volume(&mut self, volume: f32) -> Result<(), AudioError> {
        if volume < MIN_VOLUME || volume > MAX_VOLUME {
            return Err(AudioError::InvalidVolume(volume));
        }
        self.master_volume = volume;
        Ok(())
    }

    /// Volume global utama saat ini.
    pub fn master_volume(&self) -> f32 {
        self.master_volume
    }

    /// Jumlah track yang dimuat.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Semua ID track.
    pub fn track_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.tracks.keys().copied().collect();
        ids.sort();
        ids
    }
}

impl AudioManager {
    /// Akses mutable track untuk keperluan tes (looping, dll.).
    #[cfg(test)]
    fn track_mut(&mut self, id: u64) -> Option<&mut AudioTrack> {
        self.tracks.get_mut(&id)
    }
}

impl AudioTrack {
    /// Atur volume untuk tes serialisasi.
    #[cfg(test)]
    fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_awal_kosong() {
        let a = AudioManager::new();
        assert!(a.is_empty());
        assert_eq!(a.len(), 0);
        assert_eq!(a.master_volume(), 1.0);
    }

    #[test]
    fn load_memperoleh_track_baru() {
        let mut a = AudioManager::new();
        let id = a.load("file:///music/song.mp3".to_string());
        assert_eq!(id, 0);
        assert_eq!(a.len(), 1);
        let track = a.track(id).unwrap();
        assert_eq!(track.source, "file:///music/song.mp3");
        assert_eq!(track.state, AudioState::Idle);
    }

    #[test]
    fn play_pause_stop_siklus() {
        let mut a = AudioManager::new();
        let id = a.load("music.mp3".to_string());

        a.play(id).unwrap();
        assert_eq!(a.track(id).unwrap().state, AudioState::Playing);

        a.pause(id).unwrap();
        assert_eq!(a.track(id).unwrap().state, AudioState::Paused);

        a.play(id).unwrap();
        assert_eq!(a.track(id).unwrap().state, AudioState::Playing);

        a.stop(id).unwrap();
        assert_eq!(a.track(id).unwrap().state, AudioState::Stopped);
    }

    #[test]
    fn play_track_sudah_playing_ditolak() {
        let mut a = AudioManager::new();
        let id = a.load("music.mp3".to_string());
        a.play(id).unwrap();
        let err = a.play(id).unwrap_err();
        assert!(matches!(err, AudioError::AlreadyPlaying { .. }));
    }

    #[test]
    fn pause_track_yang_tidak_playing_ditolak() {
        let mut a = AudioManager::new();
        let id = a.load("music.mp3".to_string());
        let err = a.pause(id).unwrap_err();
        assert!(matches!(err, AudioError::InvalidState { .. }));
    }

    #[test]
    fn operasi_track_tidak_ditemukan() {
        let mut a = AudioManager::new();
        assert!(a.play(99).is_err());
        assert!(a.pause(99).is_err());
        assert!(a.stop(99).is_err());
        assert!(a.set_volume(99, 0.5).is_err());
    }

    #[test]
    fn volume_diluar_rentang_ditolak() {
        let mut a = AudioManager::new();
        let id = a.load("music.mp3".to_string());
        assert!(a.set_volume(id, -0.1).is_err());
        assert!(a.set_volume(id, 1.1).is_err());
        assert!(a.set_volume(id, 0.5).is_ok());
        assert_eq!(a.track(id).unwrap().volume, 0.5);
    }

    #[test]
    fn master_volume_diluar_rentang_ditolak() {
        let mut a = AudioManager::new();
        assert!(a.set_master_volume(2.0).is_err());
        assert!(a.set_master_volume(0.3).is_ok());
        assert_eq!(a.master_volume(), 0.3);
    }

    #[test]
    fn track_looping() {
        let mut a = AudioManager::new();
        let id = a.load("loop.mp3".to_string());
        let track = a.track_mut(id).unwrap();
        track.looping = true;
        assert!(a.track(id).unwrap().looping);
    }

    #[test]
    fn audio_track_serialisasi_json() {
        let track = AudioTrack::new(1, "music.mp3").with_volume(0.7);
        let json = serde_json::to_string(&track).unwrap();
        assert!(json.contains("music.mp3"));
        assert!(json.contains("\"id\":1"));
    }

    #[test]
    fn audio_error_tampilkan() {
        let e = AudioError::NotFound(3);
        assert_eq!(e.to_string(), "track tidak ditemukan: 3");
        let e2 = AudioError::AlreadyPlaying(1);
        assert_eq!(e2.to_string(), "track sudah diputar: 1");
        let e3 = AudioError::InvalidVolume(1.5);
        assert_eq!(e3.to_string(), "volume di luar rentang 0.0..=1.0: 1.5");
    }

    #[test]
    fn track_ids_terurut() {
        let mut a = AudioManager::new();
        a.load("c.mp3");
        a.load("a.mp3");
        a.load("b.mp3");
        let ids = a.track_ids();
        assert_eq!(ids, vec![0, 1, 2]);
    }
}
