use arunika_apk_loader::ApkReport;
use arunika_compat_db::CompatibilityReport;
use arunika_compat_db::DoctorFlag;

pub fn print_info(report: &ApkReport) {
    println!("Paket:          {}", report.package_name);
    println!("Versi:          {}", report.version_name);
    println!("Versi kode:     {}", report.version_code);
    println!("Target SDK:      {}", report.target_sdk);
    println!("Min SDK:        {}", report.min_sdk);
    println!("File size:      {} byte", report.file_size);
    println!("MIME:           {}", report.mime_type);
    println!("Jar seal:       {}", report.jar_seal);
    println!("Multi DEX:     {}", report.is_multidex);
    println!(
        "Debuggable:    {}",
        report.debuggable.as_deref().unwrap_or("tidak-diketahui")
    );
    if let Some(label) = &report.application_label {
        println!("Nama aplikasi:  {}", label);
    }
    if report.features.is_empty() {
        println!("Fitur deklarasi: (tidak ada)");
    } else {
        println!("Fitur deklarasi ({} entri):", report.features.len());
        for feat in &report.features {
            println!("  - {}", feat);
        }
    }
    if report.libraries.is_empty() && report.native_libraries.is_empty() {
        println!("Pustaka deklarasi: (tidak ada)");
    } else {
        if !report.libraries.is_empty() {
            println!("Pustaka (uses-library, {} entri):", report.libraries.len());
            for lib in &report.libraries {
                println!("  - {}", lib);
            }
        }
        if !report.native_libraries.is_empty() {
            println!(
                "Pustaka native (uses-native-library, {} entri):",
                report.native_libraries.len()
            );
            for lib in &report.native_libraries {
                println!("  - {}", lib);
            }
        }
    }
    println!("Komponen yang dideklarasikan:");
    println!("  Activity:   {}", report.activities.len());
    println!("  Service:    {}", report.services.len());
    println!("  Receiver:   {}", report.receivers.len());
    println!("  Provider:   {}", report.providers.len());
    if report.permissions.is_empty() {
        println!("Izin:           (tidak ada yang dideklarasikan)");
    } else {
        println!("Izin:");
        for perm in &report.permissions {
            println!("  - {}", perm);
        }
    }
}

pub fn print_compatibility(report: &CompatibilityReport) {
    let _flag_type_is_used: Option<&DoctorFlag> = report.flags.first();

    println!("Skema laporan: {}", report.schema_version);
    println!("Laporan kompatibilitas: {}", report.package_name);
    println!("Status DEX: {}", report.dex_status);
    println!("Status framework: {}", report.framework_status);
    println!("Status native: {}", report.native_status);
    println!("API yang belum didukung:");
    if report.framework_coverage.groups.is_empty() {
        println!("  (tidak ada yang ditemukan dalam cakupan uji)");
    } else {
        for group in &report.framework_coverage.groups {
            if group.unavailable.is_empty() {
                continue;
            }
            print!("  - [{}::", group.group);
            let mut first = true;
            for api in &group.unavailable {
                if !first {
                    print!(", ");
                }
                first = false;
                print!("{}", api);
            }
            println!("]");
        }
    }
    if report.flags.is_empty() {
        println!("Tanda peringatan: (tidak ada)");
    } else {
        println!("Tanda peringatan:");
        for flag in &report.flags {
            println!("  - [{}] {}", flag.code, flag.summary);
        }
    }
    println!("Catatan:");
    for line in &report.notes_lines {
        println!("  - {}", line);
    }
}
