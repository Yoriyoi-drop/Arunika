# Changelog

Format mengikuti [Keep a Changelog](https://keepachangelog.com/id/1.1.0/).
Versi mengikuti [Semantic Versioning](https://semver.org/lang/id/).

## [Unreleased] — Fase 1

### Diperbaiki

- `Activity.setContentView` kini mengisi dimensi jendela virtual default
  (1920x1080) bila belum ditetapkan, sehingga bitmap offscreen tidak
  menjadi 1x1 dan `run` melaporkan dimensi jendela yang benar.
- Resolusi resource (`Resources.getIdentifier`) kini menangani nama tanpa
  ekstensi pada entri `res/` yang memiliki ekstensi berkas, memilih entri
  dasar sebelum varian berkualifier, serta `getResourceEntryName` menghapus
  ekstensi berkas.
- `Resources.getString`/`getText` kini membaca resource mentah sebagai teks
  alih-alih melempar "tabel resource belum diurai".

### Ditambahkan

- Aset APK: `Context.getAssets()` dan `AssetManager` (`open`, `open(String,I)`,
  `list`, `getLocales`, `close`) membaca entri `assets/` langsung dari arsip
  lewat kontrak baru `AssetSource`. Aset yang tidak ada dilaporkan sebagai
  `FileNotFoundException` yang dapat ditangkap aplikasi, dan `openFd` ditolak
  eksplisit karena runtime tidak membuka deskriptor berkas APK.
- Uji stabilitas interpreter DEX (`crates/dex-engine/tests/stability.rs`): nilai wide (long/double), referensi null (`iget`, `array-length` pada null), batas array (`ArrayIndexOutOfBounds`, `NegativeArraySize`), `MAX_ARRAY_LEN`, inisialisasi kelas `<clinit>` (jalan otomatis sekali), batas langkah `MAX_STEPS`, dan eksepsi try/catch (uncaught + catch-all).
- Uji ketahanan parser DEX (`crates/dex-engine/tests/hardening.rs`): penolakan code_item/class_def/proto_ids dengan offset luar berkas, dan mutasi byte pada header tanpa panic.
- `arunika-dex-engine`: opcode `invoke-virtual/range` (0x73) ditangani secara tepat; helper `invoke_virtual_range` di test-fixtures menggunakan opcode 0x73 (sebelumnya salah menggunakan 0x74 / `invoke-super/range`).
- Tes regresi: 16 tes baru (12 stabilitas interpreter + 4 hardening parser), total 414 tes lolos lewat `cargo nextest run`.
- Tes integrasi domain crate (fase 2): `arunika-sandbox-manager` (kebijakan, ConfinedRoot, PermissionPolicy, MemoryAssetSource, asset_children), `arunika-resource-manager` (tabel resource, ID sintetis, duplikat, nama tidak valid), `arunika-activity-runtime` (lifecycle, Intent, TaskStack). Total 439 tes lolos lewat `cargo nextest run`.
- Resource APK: `Resources.getIdentifier`/`openRawResource`/
  `getResourceTypeName`/`getResourceEntryName` membaca entri `res/` langsung
  dari arsip lewat `AssetSource` dengan prefiks, sehingga resource mentah
  dapat diakses tanpa tabel resource (`resources.arsc`) yang belum diurai.
  ID resource disintesis dari posisi entri (`entries()` + 1) dan stabil untuk
  satu APK; aplikasi yang memanggil `openRawResource` dengan id hasil
  `getIdentifier` mendapatkan aliran berisi bytes asli resource tersebut.
- Loop utama kooperatif: `Looper.loop()` melaporkan _main loop idle_ lewat
  sentinel `MAIN_LOOP_IDLE` alih-alih kembali segera. Aplikasi yang menulis
  `while (true) { Looper.loop(); }` tidak lagi menjadi busy-loop yang membakar
  batas langkah (mcgg.apk turun dari ~2 juta langkah ke ~1.500). `run`
  menghentikan eksekusi di titik itu dan melaporkan `main_loop_idle`, bukan
  `ExecutionLimit`.
- `java.lang.Thread`: `getDefaultUncaughtExceptionHandler` /
  `getUncaughtExceptionHandler` sekarang mengembalikan objek
  `Thread$UncaughtExceptionHandler` kosong (bukan null) agar kode aplikasi
  tidak mendapat `NullReference` saat memakainya; `uncaughtException`
  ditangani sebagai no-op.
- Jendela offscreen: `Activity.getWindow()`/`setContentView` dan
  `android.graphics.Canvas`/`Bitmap` (`createBitmap`, `drawColor`, `drawRGB`,
  `drawRect`, `drawLine`, `drawText`, `drawBitmap`) merekam operasi gambar ke
  framebuffer RGBA di memori. `run` melaporkan keadaan jendela (`window`:
  hadir, dimensi, jumlah gambar) sehingga penggunaan antarmuka aplikasi terlihat
  tanpa grafis host.
- Lokasi loop: galat `ExecutionLimit` kini membawa method terdalam yang
  sedang berjalan (`loop_location` pada JSON), sehingga `run` menjelaskan di
  mana aplikasi berputar alih-alih hanya berapa langkah yang terpakai.
- Inisialisasi class otomatis: `<clinit>` dijalankan sekali saat class
  dipakai lewat `new-instance`, akses field statis, atau invoke statis, dengan
  penjaga rekursi untuk initializer yang memakai class-nya sendiri.
- Indeks `code_item_by_method` sehingga pencarian badan method menjadi O(1);
  sebelumnya setiap invoke memindai seluruh class def dan menjadi penyebab
  utama `run` terasa menggantung pada loop utama aplikasi.
- Eksepsi native dapat ditangkap kode aplikasi: `NativeException` mengubah
  kegagalan resolver menjadi objek `Throwable` Java, dengan hierarki
  `Throwable`/`Exception`/`RuntimeException`/`IOException` dan turunannya.
- `java.util.Objects`: `equals`, `hashCode`, `toString`, `requireNonNull`,
  `isNull`, `nonNull`, dan `deepEquals` untuk perbandingan dan null-check.
- `java.util.Arrays`: `equals`, `deepEquals`, `toString`, `fill`, `copyOf`,
  dan `sort` untuk operasi array objek.
- `arunika-dex-engine`: pola `finally` (catch-all + rethrow) diverifikasi
  dengan tes fokus.
- `arunika-view-engine`: metode `layout` yang menata hierarki view menjadi
  batas (posisi/ukuran) dengan strategi linear vertikal sederhana.
- `arunika-framework-api`: `setContentView`/`addView` kini mendaftarkan
  view ke hierarki, menjalankan tata letak, dan merender batas view ke
  framebuffer offscreen.
- `arunika-host-linux`: `OffscreenSurface` untuk mempresentasikan dan
  memvalidasi framebuffer offscreen (irisan pertama integrasi host).
- `arunika-host-linux`: `HostWindow` (model) untuk siklus hidup jendela dan
  presentasi frame dengan validasi ukuran.
- `arunika-host-linux`: ekspor frame offscreen ke BMP 32-bit lewat
  `OffscreenSurface::write_bmp` dan `HostWindow::write_snapshot`.
- `arunika-host-linux`: `MinifbWindow` backend nyata (X11/Wayland) untuk
  mempresentasikan framebuffer offscreen ke display server.
- `arunika-native-bridge`: registrasi dan pemanggilan method native berbasis
  closure melalui `JniEnv::daftarkan_native`/`panggil_native`.
- `arunika-framework-api`: jembatan `register_native_method`/
  `call_native_method` ke JVM model.
- `arunika-cli`: input keyboard kini mengirim `ACTION_DOWN` dan `ACTION_UP`,
  serta memanggil `onKeyUp` bila `dispatchKeyEvent` tidak dioverride.
- `arunika-cli`: opsi `--snapshot <path>` menulis snapshot jendela offscreen
  ke BMP melalui `HostWindow`.
- `arunika-cli`: opsi `--present` membuka jendela host nyata (minifb) dan
  menampilkan frame offscreen.
- `arunika-sandbox-manager`: `PermissionPolicy` untuk izin manifest, plus
  uji penolakan izin yang tidak dideklarasikan.
- `arunika-host-linux`: pembacaan/penulisan berkas memakai `O_NOFOLLOW`
  dan pemeriksaan berkas biasa untuk menghindari symlink race.
- `java.lang.reflect`: `Class.getMethod`/`getDeclaredMethod` mengembalikan
  objek `Method`, dan `Method.invoke` masuk kembali ke interpreter dengan
  argumen yang di-unbox.
- Timer, Handler, dan Executor kooperatif: `java.util.Timer`/`TimerTask`,
  `android.os.Handler`/`Looper`, serta
  `Executors`/`ExecutorService`/`Future` mencatat tugas lalu menjalankannya
  sekali setelah lifecycle lewat `run_due_timers`, `run_due_handlers`, dan
  `run_due_executors`.
- Berkas teks: `FileReader`, `FileWriter`, `BufferedReader`, dan
  `BufferedWriter` memakai sandbox nama berkas yang sama seperti aliran byte,
  dengan isi UTF-8 dan `readLine`/`newLine`.
- `java.security.MessageDigest` untuk MD5, SHA-1, SHA-256, dan SHA-512, serta
  `java.math.BigInteger` presisi arbitrer (aritmetika, perbandingan, shift,
  `toByteArray`).
- `String.format` (termasuk `%040x` untuk BigInteger) dan
  `replaceAll`/`replaceFirst` berbasis mesin regex Rust.
- Layanan sistem: `getSystemService`, `WindowManager`/`Display` virtual
  1920x1080@320dpi, `WifiManager`/`WifiInfo`, `TelephonyManager`,
  `ContentResolver`, `Settings$Secure` (nilai `android_id` diturunkan dari
  nama paket), dan `Resources`.
- Pemeriksaan izin `Context.checkCallingOrSelfPermission`/`checkPermission`:
  izin yang dideklarasikan manifest dianggap diberikan, izin lain ditolak.
- `ReentrantLock`/`Condition` sebagai no-op pada runtime satu thread,
  `WeakReference`, dan `LocalSocket` yang melempar `IOException` eksplisit
  alih-alih berpura-pura tersambung.
- Opcode `invoke-super` (0x6f) beserta varian `/range`, dan
  `Object.hashCode`/`equals`/`toString` berbasis identitas heap.
- Keluaran `run` menambahkan `timers_executed`, `handlers_executed`,
  `executors_executed`, dan `main_loop_detected`; batas langkah pada loop
  utama dilaporkan sebagai status, bukan disembunyikan sebagai sukses.

- Interpreter DEX subset: const, move, move-result, aritmetika int, percabangan,
  return, dan invoke direct/static/virtual di dalam DEX yang sama.
- Model nilai runtime (`Value::Int`/`Value::Ref`) dan heap objek sederhana
  dengan string serta instance tanpa field.
- Instruksi `const-string`, `new-instance`, `return-object`, dan
  `check-cast` (tanpa pemeriksaan tipe).
- Eksepsi DEX: `throw`, `move-exception`, dan try/catch dari `code_item`.
  Handler dicocokkan lewat rantai superclass pada `class_defs` sehingga
  `catch` pada tipe induk menangkap subclass, dan `catch_all` didukung.
- Eksepsi yang dilempar method lain merambat ke pemanggil dan dapat ditangkap
  handler milik pemanggil (`FrameExit::Throw`), bukan langsung menjadi error.
- `DexError::UncaughtException` dan `DexError::ThrowNonReference` untuk
  eksepsi yang tidak tertangkap dan nilai non-referensi yang dilempar.
- Akses field: `sget`/`sput` (statis per `field_ids`) dan `iget`/`iput`
  (instance disimpan per objek), dengan nilai awal nol untuk field yang belum
  diisi.
- Array: `new-array`, `array-length`, `aget`/`aput` (semua varian 23x),
  dengan batas `MAX_ARRAY_LEN` (1.048.576 elemen).
- `DexError::FieldNotFound`, `NegativeArraySize`, `ArrayIndexOutOfBounds`,
  dan `NullReference` untuk kesalahan akses field/array.
- Fixture DEX kini membangun `field_ids` dan `class_data` field statis/instance.
- Tipe numerik 64-bit dan float: `const-wide/16`, `const-wide/32`,
  `const-wide`, `const-wide/high16`, `move-wide*`, `return-wide`,
  `move-result-wide`.
- Aritmetika `long` (termasuk shift 64-bit), `float`, dan `double`; `cmp-long`,
  `cmpl/cmpg-float`, `cmpl/cmpg-double` dengan perilaku NaN sesuai DEX.
- Konversi `int`/`long`/`float`/`double` dan penyempitan `int-to-byte/char/short`.
- `Value::Wide` menyimpan bit mentah 64-bit; interpretasi long atau double
  ditentukan opcode, bukan tag nilai.
- Perintah `arunika run [--json] --class <descriptor> --method <nama> <berkas>`
  yang menjalankan satu method statis dari APK/XAPK/APKM atau berkas DEX,
  memilih DEX yang memuat method tersebut, dan melaporkan jumlah langkah
  serta nilai kembali.
- `ExecutionOutcome::steps` sehingga pemanggil tahu berapa instruksi
  dieksekusi.
- Keluaran `run` memakai `schema_version: 1` dan mengklasifikasikan nilai
  kembali menurut tipe deklarasi method (`int`, `long`, `float`, `double`,
  `string`, `object`, `array`, `ref`).
- Opcode tambahan untuk bytecode nyata: `instance-of`, `check-cast` dengan
  pemeriksaan tipe (melempar `ClassCastException` bila tidak cocok),
  `invoke-interface`, `filled-new-array`, `filled-new-array/range`,
  `fill-array-data`, serta `monitor-enter`/`monitor-exit` sebagai no-op.
- `DexError::ArrayDataMismatch` dan `InvalidArrayPayload` untuk payload
  `fill-array-data` yang tidak konsisten.
- `NativeResolver::invoke_described` menerima heap dan signature method
  (mis. `(J)J`) sehingga resolver dapat membedakan overload dan membuat
  objek baru. Implementasi bawaan tetap meneruskan ke `invoke_static`.
- `DexFile::method_signature` dan `method_parameter_types` untuk membentuk
  signature dan mengetahui lebar parameter.
- API framework bertambah: `String.length/isEmpty/charAt/equals/concat/substring/indexOf`,
  `Integer.parseInt/toString`, serta overload `Math.abs/max/min` untuk
  `int`, `long`, `float`, `double` plus `Math.sqrt` dan `Math.round`.
- Kontrak `NativeResolver` di `dex-engine`; `framework-api` mengimplementasikan
  subset pertama (`Math.abs`, `Math.max`, `Math.min`) tanpa membuat dependensi
  melingkar.
- Batas eksekusi: `MAX_STEPS` (1.000.000 langkah) dan `MAX_CALL_DEPTH` (64).
- Crate fixture kini membangkitkan DEX lengkap (string, type, proto, method,
  class_data, code_item) beserta helper encoding instruksi.
- `DexEngine::execute_method_with_args` untuk menguji method berparameter.
- `arunika run <berkas>` tanpa opsi tambahan: memilih launcher activity dari
  manifest (`action.MAIN` + `category.LAUNCHER`), menjalankan `<clinit>`
  kelasnya, lalu membuat instance activity lewat `DexEngine::instantiate_class`
  (alokasi objek + `<init>` DEX atau resolver native).
- `ApkReport::launchable_activities` diisi dari `apk-info`
  (`get_main_activities`) sehingga CLI tidak perlu menebak nama activity.
- `arunika run --args "1,teks"` mengubah argumen sesuai descriptor parameter
  (`I`, `J`, `F`, `D`, `Z`, `Ljava/lang/String;`) sebelum method dijalankan.
- `arunika run` menjalankan `<clinit>` kelas target lebih dulu agar field
  statis terinisialisasi sebelum method dipanggil.
- API framework bertambah: `StringBuilder` (`<init>`, `append(String/I/J/C/Z)`,
  `toString`, `length`) dengan teks disimpan di heap, dan
  `System.arraycopy` dengan pemeriksaan batas.
- Konstruktor dasar `android.app.Activity` disediakan runtime (no-op), sehingga
  `<init>` launcher activity pada APK nyata yang memanggil konstruktor induk
  dapat dijalankan.
- `arunika run` menjalankan lifecycle minimal setelah instance dibuat:
  `onCreate(Bundle)` (dengan `null`), lalu `onStart()` dan `onResume()` bila
  method tersebut ada di DEX. Keluaran teks menampilkan baris `Lifecycle` dan
  JSON menambahkan `lifecycle` berisi nama method yang dijalankan.
- `android.app.Activity` menerima `onCreate`/`onStart`/`onResume`/`onPause`/
  `onStop`/`onDestroy`/`onRestart` sebagai no-op; method lain tetap dilaporkan
  tidak tersedia.
- Opcode `const-class` dengan objek `java.lang.Class` yang di-cache per
  `type_ids`, plus `Class.getName()`/`getSimpleName()` di resolver.
- `Object::Class` dan `DexEngine::classes` agar `const-class` untuk tipe yang
  sama mengembalikan referensi yang sama.
- Tes ketahanan baru: mutasi byte pada instruksi DEX valid tidak boleh panic
  saat dieksekusi; pengujian berjalan di thread ber-stack 512 KiB agar batas
  kedalaman panggilan ikut teruji.
- Koleksi `java.util` dasar: `ArrayList`/`LinkedList` (`<init>`, `add`, `get`,
  `set`, `remove`, `size`, `isEmpty`, `clear`) dan `HashMap`/`LinkedHashMap`/
  `ConcurrentHashMap` (`<init>`, `put`, `get`, `remove`, `containsKey`, `size`,
  `isEmpty`, `clear`) dengan perbandingan kunci berbasis isi untuk String dan
  nilai boxed.
- Boxing `Integer.valueOf`/`intValue`/`longValue`/`toString` lewat objek
  `Object::Boxed`.
- `android.util.Log` (`v/d/i/w/e/wtf`) mengembalikan panjang pesan, serta
  `isLoggable`.
- `java.io.File` dengan operasi path (`<init>`, `getPath`, `getAbsolutePath`,
  `getAbsoluteFile`, `getName`, `getParent`, `toString`) dan pelaporan
  `exists`/`mkdirs`/`length` bahwa belum ada akses disk.
- `Context`/`Activity`: `getPackageName`, `getApplicationContext`,
  `getBaseContext`, `getFilesDir`, `getCacheDir`, `getExternalFilesDir`,
  `getExternalCacheDir`, dan `getSharedPreferences` dengan direktori aplikasi
  virtual dari `XDG_DATA_HOME`/`HOME`.
- `SharedPreferences` dan `Editor` disimpan di memori per nama, dengan
  `getString/getInt/getBoolean/getLong/getFloat`, `put*`, `remove`, `clear`,
  `commit`/`apply`.
- `Object.getClass()` mengembalikan `java.lang.Class` sesuai tipe objek.
- Opcode `move-object`, `move-object/from16`, `move-object/16`, serta
  `invoke-virtual/super/direct/static/interface-range` (format 3rc).
- Opcode `const-string/jumbo` (format 31c) dengan indeks string 32-bit.
- `NativeResolver::field_default` sebagai jembatan field framework yang
  nilainya dipasok runtime, bukan ditulis kode aplikasi.
- `android.content.pm.PackageManager` sebatas metadata manifest:
  `getPackageInfo`, `getApplicationInfo`, `getInstalledPackages`, dan
  `getPackageName`. `PackageInfo`/`ApplicationInfo` adalah objek kosong yang
  field-nya diisi lewat `field_default` (`versionName`, `versionCode`,
  `packageName`, `targetSdkVersion`, `minSdkVersion`, `metaData`).
- `AppManifest` sebagai wadah metadata manifest yang diberikan pemanggil ke
  resolver; nilai yang tidak terbaca sebagai angka dibiarkan kosong.
- `android.os.Bundle` sebagai map kunci-nilai dalam memori dengan `put*`,
  `get*` bertipe (termasuk overload nilai bawaan), `containsKey`, `isEmpty`,
  `size`, `remove`, dan `clear`.
- `ApplicationInfo.metaData` berisi pasangan `<meta-data>` dari manifest;
  nilai berbentuk teks numerik diterima oleh `Bundle.getInt/getLong`.
- `String.contentEquals(CharSequence)` yang menerima `String` maupun
  `StringBuilder` sebagai argumen.
- `ApkReport.meta_data` berisi pasangan `name`/`value` `<meta-data>`, dibaca
  per elemen manifest agar tetap berpasangan meski ada entri `android:resource`.
- Kontrak `ConfinedFileSystem` di `arunika-sandbox-manager`: nama berkas wajib
  satu komponen biasa (tanpa pemisah path, `..`, atau NUL), `ConfinedRoot`
  menolak symlink, dan ukuran berkas dibatasi `MAX_APP_FILE_BYTES` (8 MiB).
- `arunika-host-linux::AppDataFileSystem` sebagai implementasi kontrak itu di
  atas direktori data aplikasi; kegagalan penyiapan direktori dilaporkan
  sebagai `HostLinuxError::StorageNotReady`.
- `Object::Stream` dan `java.io.FileInputStream`/`FileOutputStream`
  (`read`, `read([B)`, `read([BII)`, `available`, `write(I)`, `write([B)`,
  `write([BII)`, `flush`, `close`), serta `Context.openFileInput`,
  `openFileOutput`, `deleteFile`, dan `fileList`.
- `NativeException` dan pengodean `@throw:<descriptor>:<pesan>`: kegagalan API
  framework dapat muncul sebagai eksepsi Java yang ditangkap `try/catch`,
  bukan berhenti sebagai `DexError::NativeFailure`.
- Hierarki kelas framework bawaan (`FileNotFoundException` → `IOException` →
  `Exception` → `Throwable`, dst.) agar `catch` pada superclass menangkap
  eksepsi yang dilempar runtime meski class_def-nya tidak ada di DEX.
- `Throwable.getMessage`/`toString`/`printStackTrace` untuk eksepsi runtime.
- `java.util.UUID` (`randomUUID` versi 4 dari entropi std, `fromString`,
  `toString`, `equals`, `hashCode`).
- `java.net.URLEncoder`/`URLDecoder` dengan aturan application/x-www-form-urlencoded.
- `java.lang.Long` (`parseLong`, `toString`, `valueOf`, `longValue`, `intValue`).
- `java.lang.Thread` minimal (`currentThread`, `getName`/`setName`, `getId`,
  `sleep` tanpa memblokir, `getStackTrace` kosong).
- `android.os.Process.myPid`/`myTid`, `android.text.TextUtils.isEmpty`/`equals`.
- `Class.forName`, `Class.getCanonicalName`, dan `Activity.getApplication`.
- `String`: konstruktor `()`/`(String)`/`([B)`/`([BII)`, `getBytes` (UTF-8),
  `startsWith`, `endsWith`, `contains`, `trim`, `toLowerCase`, `toUpperCase`,
  `replace`, dan `compareTo`.
- Opcode binop format 12x `0xb0..=0xcf` (`add-int/2addr` sampai
  `rem-double/2addr`), termasuk pembagian dengan pemeriksaan pembagi nol.
- `ARUNIKA_TRACE` kini juga mencetak nilai kembalian API native.

### Diubah

- Interpreter dipecah menjadi `step` (satu instruksi) dan `run_frame`
  (kendali frame). Semua cabang opcode kini berada di `step`, sehingga frame
  yang bertumpuk saat rekursi kecil dan rekursi bytecode berhenti karena
  `MAX_CALL_DEPTH`, bukan karena stack overflow. Diuji di thread dengan stack
  256 KiB.

### Diperbaiki

- `type_list` pada `proto_ids` dibaca sebagai 4 byte per tipe padahal DEX
  memakai `type_item` 2 byte. Akibatnya parameter method dengan dua argumen
  atau lebih terbaca kosong, sehingga jumlah argumen dan pencocokan overload
  salah. Fixture ikut diperbaiki agar menulis 2 byte per tipe.
- Encoder `binop` fixture menukar register tujuan dan operand pertama, sehingga
  tes aritmetika tidak menguji register yang dimaksud.
- `DexError::NoCode` kini menyertakan signature method agar API yang belum
  tersedia dapat dikenali persis (mis. `Landroid/util/Log;#d(...)I`).
- `if-eq`/`if-ne` kini membandingkan nilai apa adanya sehingga perbandingan
  referensi objek bekerja; `if-eqz`/`if-nez` memperlakukan `Int(0)` sebagai
  `null` dan referensi objek sebagai bukan-null. Sebelumnya keduanya membaca
  register sebagai int dan gagal dengan `TypeMismatch` pada referensi.
- Pemanggilan method kini menyertakan `this` implisit untuk method instance
  dan melewati word kedua argumen `long`/`double`, sehingga jumlah argumen
  yang diterima resolver sesuai dengan parameter method.
- Offset cabang DEX dihitung relatif terhadap instruksi berikutnya, padahal
  spesifikasi menyatakan relatif terhadap instruksi cabang itu sendiri.
  Semua `goto`, `if-*`, dan `fill-array-data` kini memakai
  `pc + offset`; fixture dan tes disesuaikan.
- Tata letak `code_item` salah: `tries_size` dibaca sebagai `outs_size`, dan
  try/catch dibaca sebelum `insns`. Akibatnya instruksi seluruh method bergeser
  dan jumlah code item salah. Struktur kini mengikuti urutan DEX yang benar
  (header, insns, lalu try/catch).
- String DEX dibaca sebagai UTF-16 padahal formatnya MUTF-8; nama class dan
  method kini terbaca benar.
- `DexFile` menyimpan peta `code_off` ke code item sehingga eksekusi tidak
  memindai ulang seluruh class_data.
- Nilai awal field kini mengikuti tipe DEX-nya: field `J`/`D` mendapat
  `Wide(0)`, bukan `Int(0)`. Sebelumnya operasi long/double berikutnya gagal
  dengan `TypeMismatch` karena lebar nilainya tidak cocok.
- `<meta-data>` tidak lagi dipasangkan lewat dua daftar atribut terpisah yang
  bisa tidak seimbang (entri `android:resource` membuat jumlahnya berbeda dan
  seluruh pasangan hilang). Manifest kini ditelusuri per elemen.

### Catatan

- Runtime berjalan pada satu thread: timer, handler, dan executor hanya
  mencatat tugas lalu menjalankannya sekali setelah lifecycle. Delay, periode,
  dan pengulangan tidak dihormati karena belum ada jam dinding atau penjadwal.
- Loop utama aplikasi memang tidak pernah selesai. Bila tidak ada tugas
  tertunda (antrean pesan kosong), runtime berhenti pada `Looper.loop()` dan
  melaporkan `main_loop_idle: true`. Bila aplikasi memang berputar tanpa
  henti tanpa mencapai `loop()`, runtime berhenti pada `MAX_STEPS`
  (1.000.000 langkah) dan melaporkan `main_loop_detected: true`, bukan
  mengklaim aplikasi selesai berjalan.
- `AssetManager` hanya membaca aset dari APK tunggal; container XAPK/APKM
  belum mengambil aset dari `base.apk` dan dilaporkan sebagai peringatan saat
  `run`.
- Jendela bersifat offscreen: operasi `Canvas`/`Bitmap` direkam ke framebuffer
  di memori dan dilaporkan oleh `run`, tetapi belum ada grafis host yang
  menampilkan jendela secara nyata. Input pengguna (sentuh/keyboard) belum ada.
- Belum ada JNI/NDK, render ke layar, maupun penyimpanan state. Konfigurasi
  jendela Activity yang tidak memengaruhi framebuffer offscreen diterima sebagai
  no-op.
- Identitas perangkat bersifat virtual: MAC placeholder, `android_id` turunan
  nama paket, dan IMEI/IMSI kosong. Tidak ada pembacaan identitas perangkat
  nyata.
- `Resources` tidak mengurai tabel resource: `getString`/`getText` melempar
  `NotFoundException` dan `getIdentifier` mengembalikan 0.
- Regex memakai mesin Rust; lookaround dan backreference ditolak dengan error,
  bukan diganti diam-diam.
- Heap tidak memiliki GC dan `WeakReference` tidak pernah dikosongkan.
- Pencocokan tipe eksepsi belum mempertimbangkan interface: `catch` pada
  interface tidak menangkap apa pun.
- `aget`/`aput` belum menormalkan lebar tipe: semua varian membaca dan menulis
  nilai 32-bit yang sama.
- Long dan double disimpan utuh pada register pertama pasangan, bukan dua word
  terpisah. Aman karena DEX selalu merujuk pasangan lewat register pertama.
- Rekursi interpreter memakai stack host dengan kedalaman maksimum 64.
- `Bundle` dan `SharedPreferences` hanya hidup di memori proses.
- Akses berkas terbatas pada satu tingkat di `<data>/files`; isi
  `FileOutputStream`/`FileWriter` baru ditulis saat `flush`/`close`.
- `java.io.File` hanya memanipulasi path; `exists`/`mkdirs`/`length`
  melaporkan tidak ada berkas.
- Lifecycle hanya dijalankan untuk method yang ada di DEX kelas activity itu
  sendiri, bukan superclass.
- `run` berhenti dengan exit code 1 dan pesan eksplisit ketika kode aplikasi
  memanggil API yang belum tersedia, bukan melanjutkan seolah berhasil.
- `Class.getName()` mengembalikan descriptor apa adanya untuk array dan tipe
  primitif.
- `finally` belum didukung sebagai handler terpisah; hanya tercakup lewat
  `catch_all`.

## [1.0.0] - 2026-10-31

Rilis stabil pertama untuk **fase 0: inspeksi APK statis**. Arunika belum
menjalankan aplikasi Android dan tidak mengklaim kompatibilitas Android.

### Ditambahkan

- Perintah `info`, `compat`, dan `doctor` dengan keluaran teks dan `--json`.
- `schema_version` pada keluaran JSON `info`, `compat`, dan `doctor`.
- Inspeksi DEX langsung dari memori (`inspect_dex_bytes`) tanpa berkas
  sementara.
- Crate `arunika-test-fixtures` yang membangkitkan APK/XAPK/APKM sintetis
  (AXML + DEX) untuk pengujian.
- Batas `MAX_DEX_BYTES` (256 MiB) dan batas jumlah entri tabel DEX.
- Validasi header DEX: ukuran header, endian tag, dan ukuran berkas.
- `LoaderError` bertipe untuk semua kegagalan `apk-loader`, menggantikan
  `String` bebas.

### Diperbaiki

- Workspace tidak dapat dikompilasi karena blok kode menggantung pada
  `apk-loader`.
- Pembacaan `classes.dex` dari APK tunggal selalu gagal; sekarang APK tunggal
  dan container XAPK/APKM sama-sama didukung.
- Daftar DEX pada container memakai nama entry dari arsip yang salah.
- `info` tidak lagi menulis dan menghapus berkas DEX sementara di `/tmp`.
- `info` kini juga menginspeksi DEX di dalam container XAPK/APKM, bukan hanya
  berkas berekstensi `.apk`.
- Menulis ke stdout yang sudah ditutup (mis. `arunika info x.apk | head`)
  tidak lagi memicu panic "failed printing to stdout"; proses berhenti karena
  SIGPIPE seperti CLI Unix pada umumnya.
- `info` tetap menampilkan metadata APK ketika DEX tidak dapat diinspeksi, dan
  melaporkan penyebabnya pada `dex_error`.

### Keamanan

- Header DEX palsu tidak dapat memicu alokasi memori raksasa.
- Parser DEX diuji terhadap pemotongan dan mutasi byte tanpa panic.
- Loader APK diuji terhadap pemotongan, mutasi byte, dan ukuran entry palsu.

### Diketahui belum tersedia

- Eksekusi DEX, Activity lifecycle, framework API, view/graphics, JNI, sandbox
  yang menegakkan isolasi, dan backend jendela Linux/macOS.
- Verifikasi tanda tangan APK.
- Dukungan Google Play Services.
