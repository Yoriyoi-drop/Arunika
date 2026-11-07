# Arunika — rancangan desain

> **Status:** fondasi fase 0 stabil (1.0.0); fase 1 (proof of concept) berjalan.
> Nama Arunika masih sementara. Workspace sudah mengeksekusi subset DEX dari APK
> nyata, tetapi belum menampilkan jendela dan tidak mengklaim kompatibilitas
> Android.

## 1. Filosofi dan sasaran

Arunika mengejar **kompatibilitas, bukan emulasi penuh**: menjalankan aplikasi
melalui runtime dan layanan yang kompatibel tanpa menjadikan VM Android penuh
sebagai mekanisme utama. Sasaran produk adalah aplikasi Android tampil sebagai
jendela desktop, dengan input, clipboard, dan integrasi launcher yang sesuai
dengan platform.

Core, pengelolaan proses, loader, IPC, lifecycle, kebijakan keamanan, dan
orkestrasi runtime ditulis dalam Rust. Komponen Android atau native upstream
boleh dipakai jika lisensi, integrasi, dan batas keamanannya dapat diterima.
Setiap aplikasi harus diisolasi; APK adalah kode tidak tepercaya dan tidak
memperoleh akses komputer host secara otomatis.

Target kompatibilitas harus berupa kumpulan aplikasi teruji dengan hasil,
crash rate, dan API yang belum didukung yang terukur. Tidak ada klaim “semua
aplikasi Android berjalan”.

## 2. Mode kompatibilitas

### Mode A — Native Compatibility Runtime (target utama)

Menjalankan DEX melalui runtime kompatibel, menyediakan subset Android
framework API, dan menerjemahkan kebutuhan aplikasi menjadi layanan host.
Mode ini berpotensi lebih ringan dan cepat serta tidak memerlukan Android OS
lengkap, tetapi implementasi runtime, framework, dan rendering merupakan
pekerjaan besar.

Jangan mengklaim dukungan API yang belum tersedia. `run` harus menjelaskan
keterbatasan yang diketahui dan hanya menawarkan fallback jika fallback itu
benar-benar didukung.

### Mode B — Android System Compatibility (fallback terbatas)

Pada Linux, opsi fallback dapat menjalankan Android userspace melalui
container, menggunakan ART dan layanan sistem yang sesuai, kemudian memetakan
aplikasi ke jendela desktop. Opsi ini membutuhkan image serta layanan sistem
dan memiliki batas kompatibilitas, lisensi, serta keamanan tersendiri.

Container Android Linux bukan fallback lintas platform yang sama untuk
macOS. Pada macOS, native runtime menjadi jalur utama; virtualisasi hanya
dipertimbangkan sebagai proyek dan keputusan produk terpisah.

## 3. Alur dan batas arsitektur

```text
APK / split APK
  -> Rust package manager dan apk-loader
  -> validasi arsip, signature, manifest, resource, DEX, ABI, dan izin
  -> pemilihan mode + compatibility core
  -> host integration API yang netral terhadap platform
  -> backend Linux (Wayland/X11, GPU, PipeWire) atau macOS (AppKit, Metal,
     CoreAudio)
  -> jendela aplikasi desktop
```

Komponen runtime dipisah menurut tanggung jawab: `apk-loader`,
`manifest-parser`, `resource-manager`, `dex-engine`, `framework-api`,
`activity-runtime`, `view-engine`, `graphics-engine`, `native-bridge`,
`service-broker`, `sandbox-manager`, `compat-db`, `host-linux`, dan
`host-macos`. Daftar ini adalah batas arsitektur yang dituju, bukan klaim bahwa
crate-crate tersebut sudah diimplementasikan.

Aturan dependency:

- CLI hanya menangani antarmuka pengguna dan mengorkestrasi use case; ia bukan
  tempat implementasi loader, runtime, framework, atau backend.
- Domain dan framework API tidak boleh bergantung pada AppKit, Wayland,
  PipeWire, atau detail backend lain.
- Platform berinteraksi melalui trait/API host yang kecil dan eksplisit.
- `host-linux` dan `host-macos` mengimplementasikan kontrak host; keduanya
  tidak saling bergantung.
- `sandbox-manager` menentukan kebijakan isolasi. Kegagalan menyiapkan
  sandbox harus ditampilkan sebagai error, bukan dilewati diam-diam.
- Dependensi antarkrate harus satu arah dan sesedikit mungkin. Hindari siklus,
  crate serba-guna, serta duplikasi kontrak.

## 4. Subsistem dan tanggung jawab

| Subsistem | Tanggung jawab |
|---|---|
| `apk-loader` | Membaca APK, memvalidasi ZIP, menangani split APK, dan menolak input berbahaya; fondasi fase 0 membungkus parser upstream `apk-info`/`apk-info-zip` (Apache-2.0). |
| `manifest-parser` | Manifest, komponen, permission, dan metadata paket. |
| `resource-manager` | Resource Android dan decoding resource. |
| `dex-engine` | DEX, register, pemanggilan method, exception, threading, dan GC. |
| `framework-api` | Implementasi Android API bertahap berdasarkan coverage terukur. |
| `activity-runtime` | Activity lifecycle, intent, task, dan focus. |
| `view-engine` | Layout, teks, widget, animasi, dan dispatch event. |
| `graphics-engine` | Canvas, rendering, dan bridging GLES/Vulkan bila tersedia. |
| `native-bridge` | JNI, ELF, simbol native, dan ABI yang didukung. |
| `service-broker` | Package Manager, clipboard, notifikasi, permission, dan layanan aplikasi. |
| `sandbox-manager` | Isolasi proses, filesystem, jaringan, permission, dan batas resource. |
| `compat-db` | Skema laporan kompatibilitas dan temuan; fondasi fase 0 belum menyimpan hasil uji persisten. |
| `host-linux` | Jendela, input, GPU, audio, dan integrasi Linux. |
| `host-macos` | Jendela, input, GPU, audio, dan integrasi macOS. |

### Strategi runtime dan framework

Jangan menulis ulang seluruh ART sebelum ada aplikasi yang berhasil dijalankan.
Evaluasi komponen Android upstream berdasarkan lisensi dan integrasi, atau
mulai dengan interpreter DEX Rust untuk subset yang teruji. Interpreter
mandiri tetap harus menangani semantik tipe/register, exception, pemanggilan
method, sinkronisasi, dan JNI. Interpreter saja tidak menjamin kompatibilitas
ART atau framework internal.

Framework dibangun per kelompok dan dibuktikan dengan test oracle:

1. `android.util`, `android.os`, `android.content`.
2. `android.app`, lifecycle Activity, intent, dan task.
3. `android.view`, input, window, dan focus.
4. `android.widget`, teks, layout, dan kontrol.
5. `android.graphics` dan resource.
6. Storage, networking, permission, dan notifikasi.
7. JNI dan Android NDK untuk ABI yang secara eksplisit didukung.

API yang belum tersedia harus tercatat di compatibility report; stub yang
diam-diam sukses bukan implementasi.

Method framework yang disediakan runtime (bukan bytecode) diakses melalui
kontrak `NativeResolver` milik `dex-engine`. `framework-api` mengimplementasikan
kontrak itu, sehingga dependensi tetap satu arah dan interpreter tidak perlu
mengenal Android API secara langsung. Cakupan saat ini: `Math`, `String` (`format`,
`replaceAll`/`replaceFirst`, `split`, `getBytes`), `Integer` (termasuk
boxing), `StringBuilder`, `Class` dan `java.lang.reflect` (`getMethod`/
`getDeclaredMethod` + `Method.invoke` kembali ke interpreter), koleksi
`java.util`, `System.arraycopy`, `java.io.File` (path saja) beserta
`FileReader`/`FileWriter`/`BufferedReader`/`BufferedWriter` di atas
`ConfinedFileSystem`, `android.util.Log`, `SharedPreferences` di memori,
`android.os.Bundle` di memori, `Timer`/`TimerTask`, `Looper`/`Handler`,
`Executors`/`ExecutorService`/`Future` (penjadwalan kooperatif),
`MessageDigest` (MD5/SHA-1/SHA-256/SHA-512), `BigInteger`, `ReentrantLock`/
`Condition`, `WeakReference`, layanan sistem (`WifiManager`,
`TelephonyManager`, `WindowManager`/`Display`, `ContentResolver`/
`Settings`, `Resources`), `AssetManager` (aset read-only dari APK),
jendela offscreen (`Window`/`Canvas`/`Bitmap` yang merekam gambar ke
framebuffer di memori), serta `Context`/`Activity` dasar (direktori aplikasi
virtual, pemeriksaan izin dari manifest, lifecycle dengan jendela offscreen).
Akses berkas tidak dilakukan langsung oleh `framework-api`:
`arunika-sandbox-manager` mendefinisikan kontrak `ConfinedFileSystem` beserta
validasi nama dan batas ukuran, `arunika-host-linux` mengimplementasikannya di
atas direktori data aplikasi, dan CLI menyuntikkannya ke resolver. Aset APK
memakai kontrak terpisah `AssetSource` (read-only, nama boleh bersarang) yang
diimplementasikan `arunika-host-linux` dengan membaca entri `assets/` dari
arsip; hanya direktori itu yang terlihat lewat `AssetManager`. Resource APK
memakai kontrak yang sama lewat `with_prefix(path, "res/")` sehingga
`Resources.getIdentifier`/`openRawResource` menjangkau bytes asli dari `res/`
tanpa `resources.arsc`; ID resource disintesis dari posisi entri per APK. Metadata
manifest tersedia lewat `PackageManager` dan diteruskan ke
field `PackageInfo`/`ApplicationInfo` dengan hook
`NativeResolver::field_default`: field framework yang tidak pernah ditulis
kode aplikasi diisi runtime, dan nilai yang tidak diketahui manifest tetap
nol/null, bukan dikarang.

## 5. Integrasi platform

**Linux:** Wayland utama dan X11 tambahan; Vulkan/OpenGL dengan software
fallback; PipeWire untuk audio dan portal desktop untuk akses yang disetujui.
Fallback container Android userspace bersifat opsional. Binder merupakan
ketergantungan kernel yang harus diperiksa saat runtime dan tidak boleh
diasumsikan aktif.

**macOS:** AppKit, Metal, dan CoreAudio. Backend dan framework netral tidak
boleh mengasumsikan primitive kernel Linux. Container Android Linux tidak
disamakan dengan dukungan macOS.

## 6. CLI dan perilaku produk

Antarmuka yang dituju:

```text
arunika install ./calculator.apk
arunika run com.example.calculator
arunika run ./calculator.apk
arunika info com.example.calculator
arunika compat com.example.calculator
arunika doctor com.example.calculator
arunika stop com.example.calculator
arunika uninstall com.example.calculator
```

- `install` memvalidasi APK sebelum mengubah data aplikasi.
- `run` memilih mode yang benar-benar tersedia dan menjelaskan alasan jika
  aplikasi tidak kompatibel.
- `doctor` menghasilkan laporan terstruktur tanpa mengeksekusi kode APK.
- `compat` dan `doctor` menyediakan keluaran JSON versi skema untuk otomasi.
- `uninstall` meminta konfirmasi sebelum menghapus data pengguna.
- Error, keterbatasan, dan kegagalan sandbox harus terlihat jelas.

Pada fondasi fase 0, `info`, `compat`, dan `doctor` menerima path APK dan
melakukan inspeksi statis. `compat`/`doctor` menyatakan bahwa eksekusi dan
kompatibilitas belum didukung; keduanya tidak boleh memberi kesan APK dapat
dijalankan. `install`, `stop`, dan `uninstall` belum tersedia.

Fase 1 menambahkan `run` versi proof of concept dengan dua bentuk. Bentuk
produksi adalah `arunika run <berkas.apk>`: launcher activity dipilih dari
manifest (`action.MAIN` + `category.LAUNCHER`), `<clinit>` kelasnya dijalankan,
lalu instance activity dibuat lewat `new-instance` + `<init>` (bytecode DEX
atau resolver native). Bentuk debug adalah
`arunika run [--json] --class <descriptor> --method <nama> [--args <nilai>] <berkas>`,
yang memilih DEX (di dalam APK/XAPK/APKM atau berkas `.dex`) yang memuat method
tersebut, menjalankannya di interpreter subset, lalu melaporkan jumlah langkah
dan nilai kembali. Setelah lifecycle selesai, tugas `Timer`, `Handler`, dan `Executor` yang
tertunda dijalankan secara kooperatif pada satu thread (tanpa paralelisme).
`Looper.loop()` yang tidak berujung dibatasi `MAX_STEPS` dan dilaporkan sebagai
`main_loop_detected`. Bila antrean pesan kosong, `Looper.loop()` melaporkan
_main loop idle_ (`main_loop_idle`) dan runtime berhenti dengan rapi alih-alih
membakar batas langkah. Kedua bentuk menyediakan jendela offscreen
(framebuffer di memori) tetapi **belum** menyediakan tampilan host, render ke
layar, input pengguna, maupun JNI.

`info` menampilkan metadata manifest dan ringkasan DEX pertama di dalam APK.
Jika DEX gagal dibaca atau diparsing, metadata tetap ditampilkan dan penyebab
dilaporkan sebagai peringatan (teks) atau `dex_error` (JSON); kegagalan DEX
tidak boleh mengubah APK yang rusak menjadi tampak sehat. Ketiga perintah
menyertakan `schema_version` pada keluaran JSON, dan pemanggil otomasi harus
menolak versi skema yang tidak dikenal.

Isi DEX dibatasi `MAX_DEX_BYTES` (256 MiB) sebelum dan sesudah dekompresi,
serta jumlah entri per tabel DEX dibatasi agar header palsu tidak memicu
alokasi raksasa. Header DEX divalidasi (magic, ukuran header, endian tag,
ukuran berkas) sebelum tabel dibaca.
`compat-db` saat ini membentuk laporan bertipe dengan versi skema dan temuan
statis; penyimpanan hasil uji kompatibilitas belum dibuat. Pembacaan
sertifikat/signature bukan verifikasi autentisitas APK. Ketiga perintah
inspeksi mendukung `--json`; hasil `info` menyertakan metadata manifest serta
daftar DEX/native library, fitur, izin, dan deklarasi aplikasi tanpa
mengekstrak atau mengeksekusi isinya.

APK tunggal dibaca langsung sebagai ZIP. Untuk format XAPK/APKM, parser
mengambil arsip `base.apk` di dalam container. Jangan memanggil API
`get_inner_zip` untuk APK tunggal: API itu hanya berlaku bagi container dan
mengembalikan “not a container” pada APK normal.

## 7. Tahapan pengembangan

| Fase | Hasil yang diharapkan |
|---|---|
| 0 — Fondasi (**stabil 1.0.0**) | APK inspection, metadata manifest/DEX/ABI/resource, CLI, corpus uji, dan laporan kompatibilitas; belum menjalankan aplikasi. |
| 1 — Proof of concept (**sedang berjalan**) | Satu aplikasi sederhana, subset DEX/API, lifecycle minimal, teks, input keyboard, dan satu jendela. Progres: parser `code_item`/string benar dan interpreter subset (aritmetika int/long/float/double, cabang, field, array, invoke internal termasuk `invoke-super`, instance-of, const-class, throw/try-catch) sudah berjalan dengan tes; `<clinit>` dijalankan otomatis saat kelas pertama dipakai (seperti JVM) dengan indeks method→code O(1); `arunika run` menyiapkan launcher activity (atau method tertentu) dari APK,
termasuk `<init>` dan lifecycle `onCreate`/`onStart`/`onResume`, lalu menjalankan tugas `Timer`/`Handler`/`Executor` secara kooperatif;
APK produksi yang memanggil API di luar cakupan berhenti dengan error eksplisit. Jendela bersifat offscreen (belum ada tampilan host). Resource mentah `res/` sudah terjangkau lewat `AssetSource`. Tata letak view dan input keyboard terprogram sudah tersedia; tampilan host nyata masih belum. |
| 2 — MVP | Resource, layout, Activity, intent dasar, storage privat, clipboard, audio, dan crash reporting. |
| 3 — Ekspansi | JNI/NDK, grafis, threading, arsitektur tambahan, dan pengujian lintas arsitektur. |
| 4 — Production | Sandbox diaudit, regresi otomatis, crash isolation, profiling, serta fallback Linux bila layak. |

Setiap fase harus memiliki kriteria penerimaan sebelum cakupan diperluas.

## 8. Pengujian dan standar kualitas

- APK rusak dan arsip berbahaya ditolak tanpa crash; fuzz parser dan boundary
  native/IPC.
- Semantik DEX dan framework diuji diferensial terhadap perangkat Android
  atau emulator resmi; nondeterminisme aplikasi diperhitungkan.
- Isolasi diuji agar aplikasi tidak membaca data aplikasi lain tanpa izin.
- Crash aplikasi tidak menjatuhkan host; ukur cold/warm start, RAM, dan waktu
  rendering.
- CI hanya mengklaim backend/platform yang benar-benar dibangun dan diuji.
- Fixture APK/XAPK/APKM dibangkitkan oleh crate `arunika-test-fixtures`
  (dev-only) agar tes tidak bergantung pada berkas biner besar yang tidak ikut
  berversi; parser diuji dengan pemotongan dan mutasi byte tanpa panic.
- Uji lisensi, distribusi komponen, dan persyaratan image upstream sebelum
  mengemasnya.

## 9. Risiko dan keputusan terbuka

- **Google Play Services:** Maps, push, dan pemeriksaan integritas tidak
  otomatis tersedia. Jangan mengklaim dukungan tanpa implementasi dan izin
  yang sesuai.
- **Native dan ARM:** DEX berjalan bukan berarti ELF ARM berjalan. Translation
  ISA adalah pekerjaan terpisah dengan biaya kompatibilitas dan performa.
- **Framework luas:** ukur API coverage dan kompatibilitas aplikasi, bukan
  jumlah stub.
- **Keamanan dan lisensi:** APK adalah kode tidak tepercaya; audit loader,
  native bridge, IPC, dan komponen upstream.
- **Nama:** sebelum publikasi, periksa merek dagang, domain, serta ketersediaan
  nama paket/identifier pada platform distribusi. Pemeriksaan ini belum
  dilakukan.
