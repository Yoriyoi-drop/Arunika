use arunika_sandbox_manager::{
    AssetSource, ConfinedFileSystem, ConfinedRoot, MAX_APP_FILE_BYTES, MAX_ASSET_BYTES,
    SandboxError, asset_children, validate_asset_name,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostLinuxError {
    #[error("backend jendela belum tersedia: {0}")]
    WindowNotReady(String),
    #[error("integrasi input gagal: {0}")]
    InputNotReady(String),
    #[error("integrasi audio gagal: {0}")]
    AudioNotReady(String),
    #[error("backend grafis belum siap: {0}")]
    GraphicsNotReady(String),
    #[error("direktori data aplikasi tidak dapat disiapkan: {0}")]
    StorageNotReady(String),
}

/// Kontrak host yang diimplementasikan di host-linux.
///
/// Kontrak ini harus tetap netral terhadap backend grafis dan input
/// di level yang lebih tinggi, jadi framework-api dan activity-runtime
/// tidak bergantung langsung pada detail platform.
#[derive(Debug, Clone, Serialize)]
pub struct LinuxPlatformCapabilities {
    pub window_backend: String,
    pub input_backend: String,
    pub audio_backend: String,
    pub graphics_backend: String,
}

impl LinuxPlatformCapabilities {
    pub fn current() -> Self {
        Self {
            window_backend: "belum-diimplementasi".into(),
            input_backend: "belum-diimplementasi".into(),
            audio_backend: "belum-diimplementasi".into(),
            graphics_backend: "belum-diimplementasi".into(),
        }
    }
}

/// Berkas aplikasi di direktori data aplikasi pada host Linux.
///
/// Semua nama divalidasi lewat ConfinedRoot, sehingga kode aplikasi tidak
/// dapat membaca atau menulis di luar direktorinya, dan ukuran berkas dibatasi
/// MAX_APP_FILE_BYTES.
#[derive(Debug, Clone)]
pub struct AppDataFileSystem {
    root: ConfinedRoot,
}

impl AppDataFileSystem {
    /// Membuat direktori data bila belum ada.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Result<Self, HostLinuxError> {
        let root = ConfinedRoot::new(root);
        std::fs::create_dir_all(root.root()).map_err(|error| {
            HostLinuxError::StorageNotReady(format!("{}: {error}", root.root().display()))
        })?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &std::path::Path {
        self.root.root()
    }
}

impl ConfinedFileSystem for AppDataFileSystem {
    fn read(&self, name: &str) -> Result<Vec<u8>, SandboxError> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;

        let path = self.root.resolve(name)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|error| SandboxError::FileIo {
                name: name.to_string(),
                message: error.to_string(),
            })?;

        let metadata = file.metadata().map_err(|error| SandboxError::FileIo {
            name: name.to_string(),
            message: error.to_string(),
        })?;
        if !metadata.is_file() {
            return Err(SandboxError::FileIo {
                name: name.to_string(),
                message: "bukan berkas biasa".to_string(),
            });
        }
        if metadata.len() > MAX_APP_FILE_BYTES as u64 {
            return Err(SandboxError::TooLarge {
                name: name.to_string(),
                limit: MAX_APP_FILE_BYTES,
            });
        }

        let mut data = Vec::new();
        file.take((MAX_APP_FILE_BYTES as u64) + 1)
            .read_to_end(&mut data)
            .map_err(|error| SandboxError::FileIo {
                name: name.to_string(),
                message: error.to_string(),
            })?;
        if data.len() > MAX_APP_FILE_BYTES {
            return Err(SandboxError::TooLarge {
                name: name.to_string(),
                limit: MAX_APP_FILE_BYTES,
            });
        }
        Ok(data)
    }

    fn write(&self, name: &str, data: &[u8]) -> Result<(), SandboxError> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        if data.len() > MAX_APP_FILE_BYTES {
            return Err(SandboxError::TooLarge {
                name: name.to_string(),
                limit: MAX_APP_FILE_BYTES,
            });
        }
        let path = self.root.resolve(name)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|error| SandboxError::FileIo {
                name: name.to_string(),
                message: error.to_string(),
            })?;

        let metadata = file.metadata().map_err(|error| SandboxError::FileIo {
            name: name.to_string(),
            message: error.to_string(),
        })?;
        if !metadata.is_file() {
            return Err(SandboxError::FileIo {
                name: name.to_string(),
                message: "bukan berkas biasa".to_string(),
            });
        }

        file.write_all(data).map_err(|error| SandboxError::FileIo {
            name: name.to_string(),
            message: error.to_string(),
        })?;
        file.flush().map_err(|error| SandboxError::FileIo {
            name: name.to_string(),
            message: error.to_string(),
        })?;
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<(), SandboxError> {
        let path = self.root.resolve(name)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(SandboxError::FileIo {
                name: name.to_string(),
                message: error.to_string(),
            }),
        }
    }

    fn list(&self) -> Result<Vec<String>, SandboxError> {
        let entries =
            std::fs::read_dir(self.root.root()).map_err(|error| SandboxError::FileIo {
                name: ".".to_string(),
                message: error.to_string(),
            })?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| SandboxError::FileIo {
                name: ".".to_string(),
                message: error.to_string(),
            })?;
            let is_file = entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false);
            if !is_file {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    fn exists(&self, name: &str) -> bool {
        self.root
            .resolve(name)
            .map(|path| path.is_file())
            .unwrap_or(false)
    }
}

/// Aset read-only yang dibaca langsung dari arsip APK.
///
/// Hanya entri di bawah `assets/` yang terlihat; sisa arsip (DEX, manifest,
/// library native) tidak pernah diekspos ke kode aplikasi lewat kontrak ini.
/// Arsip dibuka ulang setiap permintaan agar tidak menyimpan deskriptor berkas
/// terbuka selama aplikasi berjalan.
#[derive(Debug, Clone)]
pub struct ApkAssetSource {
    path: std::path::PathBuf,
    /// Prefiks entri arsip yang dilayani (default "assets/"). Diubah menjadi
    /// "res/" untuk membaca resource mentah dari APK.
    prefix: String,
}

impl ApkAssetSource {
    /// Sumber aset read-only dari entri "assets/" di dalam arsip APK.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Result<Self, HostLinuxError> {
        Self::with_prefix(path, "assets/")
    }

    /// Sumber entri read-only dari prefiks tertentu di dalam arsip APK, misalnya
    /// "res/" untuk resource mentah.
    pub fn with_prefix(
        path: impl Into<std::path::PathBuf>,
        prefix: &str,
    ) -> Result<Self, HostLinuxError> {
        let path = path.into();
        if !path.is_file() {
            return Err(HostLinuxError::StorageNotReady(format!(
                "arsip APK tidak ditemukan: {}",
                path.display()
            )));
        }
        let source = Self {
            path,
            prefix: prefix.to_string(),
        };
        let archive = source
            .archive()
            .map_err(|error| HostLinuxError::StorageNotReady(error.to_string()))?;
        // Container XAPK/APKM menyimpan APK di dalam arsip; aset harus dibaca
        // dari base.apk, yang belum ditangani di sini. Tolak secara eksplisit
        // daripada melaporkan setiap aset sebagai tidak ditemukan.
        if !archive
            .file_names()
            .any(|name| name == "AndroidManifest.xml")
        {
            return Err(HostLinuxError::StorageNotReady(
                "arsip bukan APK tunggal: aset container XAPK/APKM belum didukung".to_string(),
            ));
        }
        Ok(source)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn archive(&self) -> Result<zip::ZipArchive<std::fs::File>, SandboxError> {
        let file = std::fs::File::open(&self.path).map_err(|error| SandboxError::FileIo {
            name: self.path.display().to_string(),
            message: error.to_string(),
        })?;
        zip::ZipArchive::new(file).map_err(|error| SandboxError::FileIo {
            name: self.path.display().to_string(),
            message: error.to_string(),
        })
    }

    /// Semua nama entri relatif terhadap prefiks arsip, terurut.
    fn names(&self) -> Result<Vec<String>, SandboxError> {
        let archive = self.archive()?;
        let mut names = Vec::new();
        for entry in archive.file_names() {
            if let Some(rest) = entry.strip_prefix(&self.prefix)
                && !rest.is_empty()
                && !rest.ends_with('/')
                && validate_asset_name(rest).is_ok()
            {
                names.push(rest.to_string());
            }
        }
        names.sort();
        Ok(names)
    }
}

/// Frame offscreen yang sudah dipresentasikan ke host.
#[derive(Debug, Clone, Serialize)]
pub struct PresentedFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub sequence: u64,
}

/// Permukaan offscreen host yang menerima framebuffer dari runtime.
///
/// Ini adalah irisan pertama integrasi host: bitmap offscreen dapat
/// "dipresentasikan" dan diinspeksi tanpa backend window nyata. Backend
/// window sesungguhnya tetap belum tersedia.
#[derive(Debug, Clone, Default)]
pub struct OffscreenSurface {
    frame: Option<PresentedFrame>,
    presented_count: u64,
}

impl OffscreenSurface {
    pub fn new() -> Self {
        Self::default()
    }

    /// Jumlah frame yang sudah dipresentasikan.
    pub fn presented_count(&self) -> u64 {
        self.presented_count
    }

    /// Frame terakhir yang dipresentasikan, bila ada.
    pub fn latest_frame(&self) -> Option<&PresentedFrame> {
        self.frame.as_ref()
    }

    /// Tulis frame terakhir ke berkas BMP 32-bit.
    ///
    /// Ini adalah irisan presentasi host yang dapat diuji tanpa display
    /// server: framebuffer offscreen diekspor ke format gambar standar.
    pub fn write_bmp(&self, path: impl AsRef<std::path::Path>) -> Result<(), HostLinuxError> {
        let frame = self
            .frame
            .as_ref()
            .ok_or_else(|| HostLinuxError::WindowNotReady("belum ada frame".to_string()))?;
        if frame.pixels.len() != (frame.width as usize) * (frame.height as usize) * 4 {
            return Err(HostLinuxError::GraphicsNotReady(
                "frame tidak konsisten".to_string(),
            ));
        }

        use std::io::Write;
        fn io_error(error: std::io::Error) -> HostLinuxError {
            HostLinuxError::WindowNotReady(format!("gagal menulis BMP: {error}"))
        }
        let mut file = std::fs::File::create(path).map_err(|error| {
            HostLinuxError::WindowNotReady(format!("gagal membuat BMP: {error}"))
        })?;
        let row_bytes = frame.width as usize * 4;
        let pixel_bytes = row_bytes * frame.height as usize;
        let file_size = 14 + 40 + pixel_bytes;

        // BITMAPFILEHEADER
        file.write_all(b"BM").map_err(io_error)?;
        file.write_all(&(file_size as u32).to_le_bytes())
            .map_err(io_error)?;
        file.write_all(&0u32.to_le_bytes()).map_err(io_error)?;
        file.write_all(&54u32.to_le_bytes()).map_err(io_error)?;

        // BITMAPINFOHEADER
        file.write_all(&40u32.to_le_bytes()).map_err(io_error)?;
        file.write_all(&(frame.width as i32).to_le_bytes())
            .map_err(io_error)?;
        file.write_all(&(frame.height as i32).to_le_bytes())
            .map_err(io_error)?;
        file.write_all(&1u16.to_le_bytes()).map_err(io_error)?;
        file.write_all(&32u16.to_le_bytes()).map_err(io_error)?;
        file.write_all(&0u32.to_le_bytes()).map_err(io_error)?;
        file.write_all(&(pixel_bytes as u32).to_le_bytes())
            .map_err(io_error)?;
        file.write_all(&2835u32.to_le_bytes()).map_err(io_error)?;
        file.write_all(&2835u32.to_le_bytes()).map_err(io_error)?;
        file.write_all(&0u32.to_le_bytes()).map_err(io_error)?;
        file.write_all(&0u32.to_le_bytes()).map_err(io_error)?;

        // BMP menyimpan baris dari bawah ke atas, kanal B,G,R,A.
        for y in (0..frame.height).rev() {
            for x in 0..frame.width {
                let i = ((y as usize * frame.width as usize) + x as usize) * 4;
                let rgba = &frame.pixels[i..i + 4];
                file.write_all(&[rgba[2], rgba[1], rgba[0], rgba[3]])
                    .map_err(io_error)?;
            }
        }
        file.flush().map_err(io_error)?;
        Ok(())
    }

    /// Presentasikan satu bitmap offscreen.
    ///
    /// Validasi ukuran piksel dilakukan sebelum state diubah, sehingga frame
    /// yang tidak konsisten tidak menggantikan frame terakhir.
    pub fn present(
        &mut self,
        bitmap: &arunika_graphics_engine::Bitmap,
    ) -> Result<(), HostLinuxError> {
        let expected = (bitmap.lebar as usize)
            .checked_mul(bitmap.tinggi as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| {
                HostLinuxError::GraphicsNotReady(format!(
                    "ukuran bitmap {}x{} overflow",
                    bitmap.lebar, bitmap.tinggi
                ))
            })?;
        if bitmap.piksel.len() != expected {
            return Err(HostLinuxError::GraphicsNotReady(format!(
                "bitmap {}x{} memiliki {} byte, seharusnya {}",
                bitmap.lebar,
                bitmap.tinggi,
                bitmap.piksel.len(),
                expected
            )));
        }
        self.presented_count += 1;
        self.frame = Some(PresentedFrame {
            width: bitmap.lebar,
            height: bitmap.tinggi,
            pixels: bitmap.piksel.clone(),
            sequence: self.presented_count,
        });
        Ok(())
    }
}

/// Jendela host (model) yang menampilkan framebuffer offscreen.
///
/// Backend window nyata (X11/Wayland) belum tersedia; struktur ini
/// memodelkan siklus hidup jendela dan presentasi frame agar dapat diuji
/// tanpa bergantung pada display server.
#[derive(Debug, Clone)]
pub struct HostWindow {
    title: String,
    width: u32,
    height: u32,
    visible: bool,
    surface: OffscreenSurface,
}

impl HostWindow {
    pub fn new(title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            visible: false,
            surface: OffscreenSurface::new(),
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn show(&mut self) {
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn presented_count(&self) -> u64 {
        self.surface.presented_count()
    }

    pub fn latest_frame(&self) -> Option<&PresentedFrame> {
        self.surface.latest_frame()
    }

    pub fn write_snapshot(&self, path: impl AsRef<std::path::Path>) -> Result<(), HostLinuxError> {
        self.surface.write_bmp(path)
    }

    /// Presentasikan frame. Frame harus sesuai ukuran jendela agar tidak
    /// terjadi distorsi diam-diam.
    pub fn present(
        &mut self,
        bitmap: &arunika_graphics_engine::Bitmap,
    ) -> Result<(), HostLinuxError> {
        if bitmap.lebar != self.width || bitmap.tinggi != self.height {
            return Err(HostLinuxError::WindowNotReady(format!(
                "ukuran frame {}x{} tidak sesuai jendela {}x{}",
                bitmap.lebar, bitmap.tinggi, self.width, self.height
            )));
        }
        self.surface.present(bitmap)
    }
}

/// Backend jendela host nyata (X11/Wayland) berbasis minifb.
///
/// Ini adalah irisan presentasi host yang sesungguhnya: jendela dibuat pada
/// display server dan framebuffer offscreen ditampilkan ke pengguna. Backend
/// ini masih minimal (satu frame, tanpa event loop penuh).
pub struct MinifbWindow {
    window: minifb::Window,
    width: u32,
    height: u32,
}

impl MinifbWindow {
    pub fn new(title: impl Into<String>, width: u32, height: u32) -> Result<Self, HostLinuxError> {
        if width == 0 || height == 0 {
            return Err(HostLinuxError::WindowNotReady(
                "ukuran jendela tidak boleh nol".to_string(),
            ));
        }
        let window = minifb::Window::new(
            &title.into(),
            width as usize,
            height as usize,
            minifb::WindowOptions::default(),
        )
        .map_err(|error| {
            HostLinuxError::WindowNotReady(format!("gagal membuat jendela: {error}"))
        })?;
        Ok(Self {
            window,
            width,
            height,
        })
    }

    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }

    pub fn present(
        &mut self,
        bitmap: &arunika_graphics_engine::Bitmap,
    ) -> Result<(), HostLinuxError> {
        if bitmap.lebar != self.width || bitmap.tinggi != self.height {
            return Err(HostLinuxError::WindowNotReady(format!(
                "ukuran frame {}x{} tidak sesuai jendela {}x{}",
                bitmap.lebar, bitmap.tinggi, self.width, self.height
            )));
        }
        let mut buffer = Vec::with_capacity((self.width * self.height) as usize);
        for chunk in bitmap.piksel.as_chunks::<4>().0 {
            let color =
                (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
            buffer.push(color);
        }
        self.window
            .update_with_buffer(&buffer, self.width as usize, self.height as usize)
            .map_err(|error| {
                HostLinuxError::WindowNotReady(format!("gagal mempresentasikan frame: {error}"))
            })?;
        Ok(())
    }
}

impl AssetSource for ApkAssetSource {
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, SandboxError> {
        validate_asset_name(name)?;
        let mut archive = self.archive()?;
        let entry_name = format!("{}{name}", self.prefix);
        let mut entry = match archive.by_name(&entry_name) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(error) => {
                return Err(SandboxError::FileIo {
                    name: name.to_string(),
                    message: error.to_string(),
                });
            }
        };
        if entry.size() > MAX_ASSET_BYTES as u64 {
            return Err(SandboxError::TooLarge {
                name: name.to_string(),
                limit: MAX_ASSET_BYTES,
            });
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        std::io::Read::read_to_end(&mut entry, &mut data).map_err(|error| {
            SandboxError::FileIo {
                name: name.to_string(),
                message: error.to_string(),
            }
        })?;
        if data.len() > MAX_ASSET_BYTES {
            return Err(SandboxError::TooLarge {
                name: name.to_string(),
                limit: MAX_ASSET_BYTES,
            });
        }
        Ok(Some(data))
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, SandboxError> {
        if !prefix.is_empty() && validate_asset_name(prefix).is_err() {
            return Err(SandboxError::PathEscape(prefix.to_string()));
        }
        Ok(asset_children(self.names()?, prefix))
    }

    fn entries(&self) -> Result<Vec<String>, SandboxError> {
        self.names()
    }
}

#[cfg(test)]
mod tests {
    use super::{ApkAssetSource, AppDataFileSystem, LinuxPlatformCapabilities};
    use arunika_sandbox_manager::{AssetSource, ConfinedFileSystem};

    #[test]
    fn current_menyatakan_belum_implementasi() {
        let caps = LinuxPlatformCapabilities::current();
        assert_eq!(caps.window_backend, "belum-diimplementasi");
        assert_eq!(caps.graphics_backend, "belum-diimplementasi");
    }

    #[test]
    fn menulis_dan_membaca_berkas_di_direktori_aplikasi() {
        let root = std::env::temp_dir().join(format!("arunika-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let fs = AppDataFileSystem::new(&root).expect("direktori data harus dibuat");

        fs.write("cache.dat", b"halo")
            .expect("tulis harus berhasil");
        assert!(fs.exists("cache.dat"));
        assert_eq!(fs.read("cache.dat").expect("baca"), b"halo");
        assert_eq!(fs.list().expect("daftar"), vec!["cache.dat".to_string()]);

        fs.remove("cache.dat").expect("hapus harus berhasil");
        assert!(!fs.exists("cache.dat"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn akses_keluar_direktori_aplikasi_ditolak() {
        let root = std::env::temp_dir().join(format!("arunika-fs-escape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let fs = AppDataFileSystem::new(&root).expect("direktori data harus dibuat");

        assert!(fs.read("../luar.dat").is_err());
        assert!(fs.write("/tmp/luar.dat", b"x").is_err());
        assert!(!fs.exists("../luar.dat"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn aset_hanya_dibaca_dari_direktori_assets_dalam_apk() {
        let apk = arunika_test_fixtures::apk_bytes_with_entries(
            &arunika_test_fixtures::AppSpec::default(),
            &[
                ("assets/boot.bin", b"halo".as_slice()),
                ("assets/config/level1.json", b"{}".as_slice()),
                ("classes.dex", b"bukan-aset".as_slice()),
            ],
        );
        let path = std::env::temp_dir().join(format!("arunika-aset-{}.apk", std::process::id()));
        std::fs::write(&path, &apk).expect("tulis apk uji");
        let assets = ApkAssetSource::new(&path).expect("arsip harus terbuka");

        assert_eq!(
            assets.read("boot.bin").expect("baca").as_deref(),
            Some(&b"halo"[..])
        );
        // Entri di luar assets/ tidak terlihat sebagai aset.
        assert!(assets.read("classes.dex").expect("baca").is_none());
        assert!(assets.read("tidak-ada.bin").expect("baca").is_none());
        assert!(assets.read("../manifest").is_err());
        assert_eq!(
            assets.list("").expect("daftar akar"),
            vec!["boot.bin".to_string(), "config".to_string()]
        );
        assert_eq!(
            assets.list("config").expect("daftar config"),
            vec!["level1.json".to_string()]
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn symlink_berkas_aplikasi_tidak_diikuti() {
        let root = std::env::temp_dir().join(format!("arunika-fs-symlink-{}", std::process::id()));
        let target = std::env::temp_dir().join(format!("arunika-fs-target-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&target);
        std::fs::create_dir_all(&root).expect("root harus dibuat");
        std::fs::write(&target, b"rahasia").expect("target harus dibuat");
        let fs = AppDataFileSystem::new(&root).expect("fs harus dibuat");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, root.join("tautan.dat")).expect("symlink harus dibuat");

        assert!(fs.read("tautan.dat").is_err());
        assert!(fs.write("tautan.dat", b"x").is_err());
        assert!(!fs.exists("tautan.dat"));

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&target);
    }
}

#[cfg(test)]
mod host_surface_tests {
    use super::OffscreenSurface;
    use arunika_graphics_engine::Bitmap;

    #[test]
    fn offscreen_surface_menolak_bitmap_tidak_konsisten() {
        let mut surface = OffscreenSurface::new();
        let mut bitmap = Bitmap::baru(1, 2, 2).expect("bitmap valid");
        bitmap.piksel.pop();
        assert!(surface.present(&bitmap).is_err());
        assert_eq!(surface.presented_count(), 0);
        assert!(surface.latest_frame().is_none());
    }

    #[test]
    fn offscreen_surface_menyimpan_frame_terakhir() {
        let mut surface = OffscreenSurface::new();
        let mut bitmap = Bitmap::baru(7, 2, 2).expect("bitmap valid");
        bitmap.piksel[0] = 42;

        surface.present(&bitmap).expect("present harus berhasil");
        assert_eq!(surface.presented_count(), 1);

        let frame = surface.latest_frame().expect("frame harus ada");
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.sequence, 1);
        assert_eq!(frame.pixels[0], 42);

        let second = Bitmap::baru(8, 1, 1).expect("bitmap valid");
        surface
            .present(&second)
            .expect("present kedua harus berhasil");
        assert_eq!(surface.presented_count(), 2);
        assert_eq!(surface.latest_frame().unwrap().width, 1);
    }
}

#[cfg(test)]
mod host_window_tests {
    use super::{HostWindow, MinifbWindow};
    use arunika_graphics_engine::Bitmap;

    #[test]
    fn host_window_menolak_frame_ukuran_tidak_sesuai() {
        let mut window = HostWindow::new("Demo", 2, 2);
        window.show();
        let bitmap = Bitmap::baru(1, 1, 1).expect("bitmap valid");
        assert!(window.present(&bitmap).is_err());
        assert_eq!(window.presented_count(), 0);
        assert!(window.latest_frame().is_none());
    }

    #[test]
    fn host_window_mempresentasikan_frame_dan_menyimpan_state() {
        let mut window = HostWindow::new("Demo", 2, 2);
        window.show();
        assert!(window.is_visible());

        let mut bitmap = Bitmap::baru(2, 2, 2).expect("bitmap valid");
        bitmap.piksel[0] = 7;
        window.present(&bitmap).expect("present harus berhasil");

        assert_eq!(window.presented_count(), 1);
        let frame = window.latest_frame().expect("frame harus ada");
        assert_eq!(frame.pixels[0], 7);
        assert_eq!(window.title(), "Demo");
        assert_eq!(window.size(), (2, 2));

        window.set_title("Baru");
        window.resize(4, 3);
        window.hide();
        assert!(!window.is_visible());
        assert_eq!(window.title(), "Baru");
        assert_eq!(window.size(), (4, 3));
    }

    #[test]
    fn host_window_menulis_snapshot_bmp() {
        let mut window = HostWindow::new("Demo", 2, 1);
        window.show();
        let mut bitmap = Bitmap::baru(3, 2, 1).expect("bitmap valid");
        bitmap.piksel[0] = 9;
        bitmap.piksel[4] = 7;
        window.present(&bitmap).expect("present harus berhasil");

        let path =
            std::env::temp_dir().join(format!("arunika-host-window-{}.bmp", std::process::id()));
        let _ = std::fs::remove_file(&path);
        window
            .write_snapshot(&path)
            .expect("snapshot harus ditulis");

        let data = std::fs::read(&path).expect("bmp harus terbaca");
        assert_eq!(&data[0..2], b"BM");
        assert_eq!(i32::from_le_bytes(data[18..22].try_into().unwrap()), 2);
        assert_eq!(i32::from_le_bytes(data[22..26].try_into().unwrap()), 1);
        // Input RGBA [9,0,0,0] ditulis sebagai BGRA [0,0,9,0].
        assert_eq!(data[54], 0);
        assert_eq!(data[55], 0);
        assert_eq!(data[56], 9);
        assert_eq!(data[57], 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn minifb_window_mempresentasikan_frame() {
        let mut window = MinifbWindow::new("Arunika Test", 4, 2).expect("jendela harus dibuat");
        assert!(window.is_open());

        let mut bitmap = Bitmap::baru(9, 4, 2).expect("bitmap valid");
        bitmap.piksel[0] = 255;
        window.present(&bitmap).expect("present harus berhasil");
    }
}
