# Arunika

Arunika adalah runtime kompatibilitas Android untuk desktop: sasaran akhirnya
menjalankan APK sebagai jendela desktop tanpa menjadikan VM Android penuh
sebagai mekanisme utama. Inti runtime, loader, kebijakan keamanan, dan
orkestrasi ditulis dalam Rust; APK diperlakukan sebagai kode tidak tepercaya.

> **Status 1.0:** produk yang stabil saat ini adalah **inspeksi APK statis**
> (fase 0) ditambah **eksekusi DEX subset** lewat `arunika run` (fase 1,
> proof of concept). Arunika **belum** menjalankan aplikasi Android dan
> **tidak** mengklaim kompatibilitas Android. Perintah `install`, `stop`, dan
> `uninstall` belum tersedia.
>
> **Fase 1 (berjalan):** interpreter DEX subset sudah ada di crate
> `arunika-dex-engine` (const, move, aritmetika int, percabangan, string,
> instance, field, array, long/float/double, invoke termasuk `invoke-super`,
> `instance-of`, `fill-array-data`, dan try/catch dengan
> `throw`/`move-exception`) dengan batas langkah, kedalaman panggilan, dan
> panjang array, plus heap objek dan kontrak `NativeResolver` yang dipenuhi
> `arunika-framework-api`. Cakupan API mencakup `Math`, `String`
> (`format`, `replaceAll`, `split`, `getBytes`), `Integer`, `StringBuilder`,
> koleksi `java.util`, `System.arraycopy`, `java.io.File`/`FileReader`/
> `FileWriter`/`BufferedReader`/`BufferedWriter`, `android.util.Log`,
> `SharedPreferences`, `Context`/`Activity` dasar (termasuk pemeriksaan izin),
> `Class` dan `java.lang.reflect` (`getMethod`/`getDeclaredMethod` +
> `Method.invoke`), `AssetManager` (aset read-only dari APK), jendela offscreen
  (`Window`/`Canvas`/`Bitmap`),
> `android.os.Bundle`, `Timer`/`TimerTask`,
> `Looper`/`Handler`, `Executors`/`ExecutorService`/`Future`,
> `MessageDigest` (MD5/SHA-1/SHA-256/SHA-512), `BigInteger`, `ReentrantLock`/
> `Condition`, `WeakReference`, layanan sistem (`WifiManager`,
> `TelephonyManager`, `WindowManager`/`Display`, `ContentResolver`/
> `Settings`, `Resources`), serta `PackageManager`/`PackageInfo`/
> `ApplicationInfo` sebatas metadata manifest. Akses berkas aplikasi dibatasi
> kontrak `ConfinedFileSystem`: hanya satu tingkat di dalam direktori data
> paket, tanpa symlink, dengan batas 8 MiB. Kegagalan API framework dapat
> muncul sebagai eksepsi Java yang ditangkap `try/catch` aplikasi.
> `<clinit>` dijalankan otomatis saat kelas pertama kali dipakai, seperti JVM.
> `arunika run aplikasi.apk` cukup dijalankan tanpa subperintah: Arunika
> memilih launcher activity dari manifest, membuat instance activity, lalu
> menjalankan `onCreate`/`onStart`/`onResume` bila ada, kemudian
> menjalankan tugas `Timer`, `Handler`, dan `Executor` yang tertunda secara
> kooperatif (satu thread, tanpa paralelisme). `Looper.loop()` yang tak
> berujung dilaporkan sebagai `main_loop_detected`, bukan kegagalan.
> APK produksi yang memanggil API di luar cakupan (mis. JNI) berhenti dengan
> pesan eksplisit dan exit code 1.
> Belum ada `finally`, JNI, atau render ke layar host; jendela bersifat
> offscreen (direkam ke framebuffer di memori).

## Kemampuan 1.0

- Membaca APK tunggal dan container XAPK/APKM (mengambil `base.apk`).
- Menampilkan metadata manifest: paket, versi, min/target SDK, label,
  `debuggable`, permission, fitur, komponen, dan daftar DEX.
- Inspeksi header DEX dan tabel utama (string/type/proto/field/method,
  class def, code item) tanpa mengeksekusi bytecode.
- Menjalankan APK lewat `arunika run <file.apk>`: launcher activity dipilih
  dari manifest, `<clinit>` dijalankan otomatis saat kelas pertama dipakai,
  instance activity dibuat, lifecycle minimal (`onCreate`, `onStart`,
  `onResume`) dijalankan bila ada, lalu tugas `Timer`/`Handler`/`Executor`
  yang tertunda dijalankan secara kooperatif. Jendela bersifat offscreen:
  operasi `Canvas`/`Bitmap` direkam ke framebuffer di memori dan dilaporkan.
- Melaporkan `main_loop_detected` bila aplikasi masuk `Looper.loop()` tanpa
  akhir (dibatasi `MAX_STEPS`), alih-alih menggantung atau gagal.
- Menjalankan satu method tertentu (opsional) lewat `--class`/`--method`
  dengan argumen bertipe (`--args`), dibatasi langkah, kedalaman panggilan,
  dan panjang array.
- Laporan kompatibilitas statis dengan kode flag eksplisit.
- Keluaran JSON berversi skema untuk otomasi.
- Penolakan input rusak tanpa panic, dengan batas ukuran DEX.

## Build

```sh
cargo build --release
```

Biner berada di `target/release/arunika`.

## Penggunaan

```text
arunika --help
arunika info   [--json] <file.apk|file.xapk|file.apkm|file.dex>
arunika compat [--json] <file.apk>
arunika doctor [--json] <file.apk>
arunika run    [--json] [--class <descriptor> --method <nama> [--args <nilai>]] <berkas>
```

`arunika run <berkas>` tanpa opsi adalah bentuk produksi: launcher activity
dipilih dari manifest. `--class`/`--method` adalah jalur debug untuk memanggil
satu method tertentu.

Contoh:

```sh
arunika info aplikasi.apk
arunika info --json aplikasi.apk | jq .package_name
arunika compat --json aplikasi.apk
arunika run aplikasi.apk
arunika run --json aplikasi.apk
arunika run --class 'Lcontoh/Hitung;' --method 'jawab' aplikasi.apk
arunika run --json --class 'Lcontoh/Hitung;' --method 'tambah' --args '2,3' aplikasi.dex
```

### Exit code

| Kode | Arti |
|---|---|
| 0 | Perintah berhasil |
| 1 | Berkas tidak dapat dibaca atau APK tidak valid |
| 2 | Argumen/perintah tidak valid atau belum tersedia |

### Skema JSON

- `info` menyertakan `schema_version: 1`, metadata manifest, `dex_entries`,
  dan objek `dex`. Jika DEX gagal diinspeksi, metadata tetap keluar dan
  kegagalan dilaporkan pada `dex_error` (bukan disembunyikan).
- `compat` dan `doctor` menghasilkan dokumen yang sama dengan
  `schema_version: 1`, status `belum diuji`, model cakupan framework, flag,
  dan catatan.
- `run` menghasilkan `schema_version: 1`, `class`, `method`, `dex`,
  `executed`, `steps`, `return_type`, dan `return_value`
  (`{"kind": "int"|"long"|"float"|"double"|"string"|"object"|"array"|"ref"}`).
- `run` tanpa opsi menghasilkan `mode: "activity"` beserta `package`,
  `activity`, `class`, `dex`, `class_initialized`, `steps`, `instantiated`,
  `lifecycle`, `timers_executed`, `handlers_executed`,
  `executors_executed`, `main_loop_detected`, `main_loop_idle`,
  `loop_location`, `window`, dan `status`.

Pemanggil otomasi harus menolak `schema_version` yang tidak dikenal.

## Batas dan keamanan

- Tidak ada eksekusi kode APK pada `info`, `compat`, maupun `doctor`.
- Aset dibaca read-only dari entri `assets/` APK dengan batas
  `MAX_ASSET_BYTES` (16 MiB) per aset; entri lain (DEX, manifest, library
  native) tidak pernah diekspos lewat `AssetManager`.
- Resource mentah dibaca read-only dari entri `res/` APK lewat `AssetSource`
  dengan prefiks; `Resources.getIdentifier`/`openRawResource` menjangkau bytes
  asli tanpa `resources.arsc`, dengan ID resource disintesis per APK.
- Isi DEX dibatasi `MAX_DEX_BYTES` (256 MiB) sebelum dan sesudah dekompresi.
- Jumlah entri per tabel DEX dibatasi untuk mencegah alokasi raksasa dari
  header palsu.
- Header DEX divalidasi: magic, ukuran header, endian tag, dan ukuran berkas.
- Pembacaan sertifikat **bukan** verifikasi autentisitas APK.
- Tidak ada klaim dukungan API Android, ABI native, atau Google Play Services.

## Arsitektur

Lihat [DESIGN.md](DESIGN.md). Aturan kontribusi ada di [AGENTS.md](AGENTS.md):
setiap crate memiliki satu tanggung jawab, dependensi satu arah, dan tidak ada
monolit.

| Crate | Tanggung jawab |
|---|---|
| `arunika` | CLI: parse argumen, orkestrasi use case, penyajian hasil |
| `arunika-apk-loader` | Membaca dan memvalidasi arsip APK/XAPK/APKM |
| `arunika-dex-engine` | Parsing DEX, interpreter bytecode, heap, dan `<clinit>` |
| `arunika-compat-db` | Laporan kompatibilitas dan flag |
| `arunika-framework-api` | Implementasi `NativeResolver` untuk API framework |
| `arunika-manifest-parser` | Metadata manifest terstruktur (belum dipakai CLI) |
| `arunika-activity-runtime` | Model lifecycle Activity (placeholder fase 0) |
| `arunika-sandbox-manager` | Kebijakan isolasi (belum menegakkan sandbox) |
| `arunika-host-linux` | Kontrak host Linux (belum diimplementasikan) |
| `arunika-test-fixtures` | Pembangkit APK/XAPK/APKM sintetis untuk tes |

## Validasi

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run
```

## Lisensi komponen upstream

`apk-info` dan `apk-info-zip` (Apache-2.0) dipakai untuk membaca arsip dan
manifest. Periksa kembali lisensi serta syarat distribusi sebelum mengemas
Arunika untuk publikasi.
