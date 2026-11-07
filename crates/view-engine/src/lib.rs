//! Mesin tampilan (view engine) untuk fase 2 MVP.
//!
//! Crate ini memiliki satu tanggung jawab: representasi hierarki View Android
//! (View, ViewGroup, TextView) dan pengaturan atribut/layout. Ia adalah crate
//! domain — tidak bergantung pada platform host (`host-linux`/`host-macos`),
//! tidak menggambar ke layar, dan tidak melakukan rendering. Pemanggilan
//! rendering diserahkan ke `graphics-engine` (fase 3).
//!
//! Inflasi layout XML dari resource APK ditangani bersama dengan
//! `resource-manager`; view-engine hanya menyediakan model data hierarki yang
//! dapat diuji secara terisolasi.

use serde::Serialize;
use std::collections::HashMap;
use thiserror::Error;

/// Kesalahan mesin tampilan.
#[derive(Debug, Error)]
pub enum ViewError {
    #[error("view tidak ditemukan: {0}")]
    NotFound(String),
    #[error("id view duplikat: {0}")]
    DuplicateId(u32),
    #[error("view bukan ViewGroup, tidak dapat menambah child: {0}")]
    NotViewGroup(String),
    #[error("atribut tidak valid: {0}")]
    InvalidAttribute(String),
}

/// ID view yang dihasilkan selama inflasi atau pembangunan hierarki.
pub type ViewId = u32;

const VIEW_ID_NONE: ViewId = 0;

/// Parameter tata letak untuk sebuah view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LayoutParams {
    /// Lebar dalam piksel (atau MATCH_PARENT/WRAP_CONTENT sentinel).
    pub width: i32,
    /// Tinggi dalam piksel (atau MATCH_PARENT/WRAP_CONTENT sentinel).
    pub height: i32,
}

impl LayoutParams {
    /// Parent mengisi seluruh ruang yang tersedia.
    pub const MATCH_PARENT: i32 = -1;
    /// Parent menyesuaikan ukuran dengan konten view.
    pub const WRAP_CONTENT: i32 = -2;

    pub fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }

    pub fn match_parent() -> Self {
        Self {
            width: Self::MATCH_PARENT,
            height: Self::MATCH_PARENT,
        }
    }

    pub fn wrap_content() -> Self {
        Self {
            width: Self::WRAP_CONTENT,
            height: Self::WRAP_CONTENT,
        }
    }
}

/// Jenis view dalam hierarki.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ViewKind {
    /// View dasar tanpa konten spesifik.
    View,
    /// Kontainer yang dapat memiliki child view.
    ViewGroup,
    /// View yang menampilkan teks.
    TextView,
}

/// Satu node dalam hierarki view.
#[derive(Debug, Clone, Serialize)]
pub struct View {
    /// ID view (0 bila tidak ditetapkan).
    pub id: ViewId,
    /// Jenis view.
    pub kind: ViewKind,
    /// Nama class view, misalnya `android.widget.TextView`.
    pub class_name: String,
    /// Atribut view (mis. `text`, `background`, `layout_width`).
    pub attributes: HashMap<String, String>,
    /// Parameter tata letak.
    pub layout_params: LayoutParams,
    /// Child view (hanya bermakna untuk ViewGroup).
    pub children: Vec<ViewId>,
}

impl View {
    pub fn new(id: ViewId, class_name: impl Into<String>, kind: ViewKind) -> Self {
        Self {
            id,
            kind,
            class_name: class_name.into(),
            attributes: HashMap::new(),
            layout_params: LayoutParams::wrap_content(),
            children: Vec::new(),
        }
    }

    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub fn with_layout_params(mut self, params: LayoutParams) -> Self {
        self.layout_params = params;
        self
    }

    /// Apakah view ini adalah ViewGroup (bisa memiliki child).
    pub fn is_view_group(&self) -> bool {
        matches!(self.kind, ViewKind::ViewGroup)
    }

    /// Ambil nilai atribut, atau `None` bila tidak ada.
    pub fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(|s| s.as_str())
    }
}

/// Hierarki view lengkap untuk satu layout.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ViewHierarchy {
    views: Vec<View>,
    id_to_index: HashMap<ViewId, usize>,
    root: Option<ViewId>,
    next_id: ViewId,
}

impl ViewHierarchy {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    /// Jumlah view dalam hierarki.
    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// View root, bila ada.
    pub fn root(&self) -> Option<&View> {
        self.root.and_then(|id| self.view_by_id(id))
    }

    /// Ambil view berdasarkan ID.
    pub fn view_by_id(&self, id: ViewId) -> Option<&View> {
        if id == VIEW_ID_NONE {
            return None;
        }
        self.id_to_index.get(&id).and_then(|i| self.views.get(*i))
    }

    /// Tambahkan view ke hierarki dan tetapkan ID baru bila ID 0 diberikan.
    ///
    /// Jika `parent` diberikan, view ditambahkan sebagai child dan parent
    /// harus merupakan ViewGroup. ID duplikat ditolak.
    pub fn add_view(
        &mut self,
        mut view: View,
        parent: Option<ViewId>,
    ) -> Result<ViewId, ViewError> {
        let id = if view.id == VIEW_ID_NONE {
            // Berikan ID baru dari counter, lalu geser counter.
            let new_id = self.next_id;
            self.next_id += 1;
            new_id
        } else {
            view.id
        };

        if self.id_to_index.contains_key(&id) {
            return Err(ViewError::DuplicateId(id));
        }

        view.id = id;

        // Validasi parent.
        if let Some(parent_id) = parent {
            let parent_view = self
                .view_by_id(parent_id)
                .ok_or_else(|| ViewError::NotFound(parent_id.to_string()))?;
            if !parent_view.is_view_group() {
                return Err(ViewError::NotViewGroup(parent_view.class_name.clone()));
            }
            // Tambahkan child ke parent (cari index parent mutably).
            let parent_index = self
                .id_to_index
                .get(&parent_id)
                .copied()
                .ok_or_else(|| ViewError::NotFound(parent_id.to_string()))?;
            self.views[parent_index].children.push(id);
        }

        let index = self.views.len();
        self.id_to_index.insert(id, index);
        self.views.push(view);

        if self.root.is_none() {
            self.root = Some(id);
        }

        Ok(id)
    }

    /// Tetapkan view root secara eksplisit.
    pub fn set_root(&mut self, id: ViewId) -> Result<(), ViewError> {
        self.view_by_id(id)
            .ok_or_else(|| ViewError::NotFound(id.to_string()))?;
        self.root = Some(id);
        Ok(())
    }

    /// Semua view, dalam urutan ditambahkan.
    pub fn views(&self) -> &[View] {
        &self.views
    }

    /// Hasil tata letak satu view: posisi dan ukuran dalam piksel.
    pub fn layout(&self, available_width: i32, available_height: i32) -> Vec<ViewBounds> {
        let mut out = Vec::new();
        if let Some(root_id) = self.root {
            self.layout_view(root_id, 0, 0, available_width, available_height, &mut out);
        }
        out
    }

    fn layout_view(
        &self,
        id: ViewId,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        out: &mut Vec<ViewBounds>,
    ) {
        out.push(ViewBounds {
            id,
            x,
            y,
            width: width.max(0),
            height: height.max(0),
        });

        let Some(view) = self.view_by_id(id) else {
            return;
        };
        if !view.is_view_group() || view.children.is_empty() {
            return;
        }

        let child_count = view.children.len() as i32;
        let mut cursor_y = y;
        for child_id in &view.children {
            let child_view = match self.view_by_id(*child_id) {
                Some(child) => child,
                None => continue,
            };
            let child_width = match child_view.layout_params.width {
                LayoutParams::MATCH_PARENT => width,
                LayoutParams::WRAP_CONTENT => width,
                other => other.max(0),
            };
            let child_height = match child_view.layout_params.height {
                LayoutParams::MATCH_PARENT => {
                    if child_count > 0 {
                        height / child_count
                    } else {
                        height
                    }
                }
                LayoutParams::WRAP_CONTENT => 48,
                other => other.max(0),
            };
            self.layout_view(*child_id, x, cursor_y, child_width, child_height, out);
            cursor_y += child_height;
        }
    }
}

/// Batas hasil tata letak untuk satu view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ViewBounds {
    pub id: ViewId,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self::wrap_content()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hierarchy() -> ViewHierarchy {
        ViewHierarchy::new()
    }

    #[test]
    fn hierarki_kosong_tidak_memiliki_view() {
        let h = hierarchy();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert!(h.root().is_none());
    }

    #[test]
    fn menambah_view_memperoleh_id_baru() {
        let mut h = hierarchy();
        let id = h
            .add_view(
                View::new(0, "android.widget.TextView", ViewKind::TextView),
                None,
            )
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(h.len(), 1);
        assert_eq!(h.root().unwrap().id, 1);
    }

    #[test]
    fn id_berurutan_lanjut_untuk_view_tanpa_id() {
        let mut h = hierarchy();
        let id1 = h.add_view(View::new(0, "A", ViewKind::View), None).unwrap();
        let id2 = h.add_view(View::new(0, "B", ViewKind::View), None).unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[test]
    fn id_disentetapas_dikeluarkan_hanya_satu_kali() {
        let mut h = hierarchy();
        let id = h
            .add_view(View::new(42, "A", ViewKind::View), None)
            .unwrap();
        assert_eq!(id, 42);
        // View berikut tanpa ID menerima 43 (next_id diambil dari id yang diberikan + ... )
        // next_id tetap 0 karena ID eksplisit digunakan; view berikutnya dapat 0 -> 1.
        let id2 = h.add_view(View::new(0, "B", ViewKind::View), None).unwrap();
        assert_eq!(id2, 1);
    }

    #[test]
    fn id_duplikat_ditolak() {
        let mut h = hierarchy();
        h.add_view(View::new(7, "A", ViewKind::View), None).unwrap();
        let err = h
            .add_view(View::new(7, "B", ViewKind::View), None)
            .unwrap_err();
        assert!(matches!(err, ViewError::DuplicateId { .. }));
    }

    #[test]
    fn menambah_child_ke_non_viewgroup_ditolak() {
        let mut h = hierarchy();
        let text_id = h
            .add_view(View::new(1, "TextView", ViewKind::TextView), None)
            .unwrap();
        let err = h
            .add_view(View::new(2, "Child", ViewKind::View), Some(text_id))
            .unwrap_err();
        assert!(matches!(err, ViewError::NotViewGroup { .. }));
    }

    #[test]
    fn child_ditambahkan_ke_viewgroup() {
        let mut h = hierarchy();
        let root_id = h
            .add_view(
                View::new(1, "LinearLayout", ViewKind::ViewGroup)
                    .with_layout_params(LayoutParams::match_parent()),
                None,
            )
            .unwrap();
        let child_id = h
            .add_view(
                View::new(2, "TextView", ViewKind::TextView).with_attribute("text", "halo"),
                Some(root_id),
            )
            .unwrap();
        let root = h.view_by_id(root_id).unwrap();
        assert_eq!(root.children, vec![child_id]);
        let child = h.view_by_id(child_id).unwrap();
        assert_eq!(child.attribute("text"), Some("halo"));
    }

    #[test]
    fn view_by_id_tidak_ditemukan() {
        let h = hierarchy();
        assert!(h.view_by_id(99).is_none());
        assert!(h.view_by_id(VIEW_ID_NONE).is_none());
    }

    #[test]
    fn set_root_mengubah_root() {
        let mut h = hierarchy();
        let _id1 = h.add_view(View::new(1, "A", ViewKind::View), None).unwrap();
        let id2 = h.add_view(View::new(2, "B", ViewKind::View), None).unwrap();
        h.set_root(id2).unwrap();
        assert_eq!(h.root().unwrap().id, id2);
    }

    #[test]
    fn set_root_id_tidak_valid() {
        let mut h = hierarchy();
        let err = h.set_root(99).unwrap_err();
        assert!(matches!(err, ViewError::NotFound { .. }));
    }

    #[test]
    fn view_dapat_diserialisasi_ke_json() {
        let view = View::new(1, "android.widget.TextView", ViewKind::TextView)
            .with_attribute("text", "halo dunia")
            .with_layout_params(LayoutParams::wrap_content());
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("TextView"));
        assert!(json.contains("halo dunia"));
    }

    #[test]
    fn hierarki_dapat_diserialisasi_ke_json() {
        let mut h = hierarchy();
        h.add_view(View::new(1, "LinearLayout", ViewKind::ViewGroup), None)
            .unwrap();
        let json = serde_json::to_string(&h).unwrap();
        assert!(json.contains("LinearLayout"));
        assert!(json.contains("ViewGroup"));
    }

    #[test]
    fn layout_params_default_adalah_wrap_content() {
        let p = LayoutParams::default();
        assert_eq!(p.width, LayoutParams::WRAP_CONTENT);
        assert_eq!(p.height, LayoutParams::WRAP_CONTENT);
    }

    #[test]
    fn layout_menghasilkan_batas_root() {
        let mut h = ViewHierarchy::new();
        h.add_view(
            View::new(1, "FrameLayout", ViewKind::ViewGroup)
                .with_layout_params(LayoutParams::match_parent()),
            None,
        )
        .unwrap();
        let bounds = h.layout(100, 200);
        assert_eq!(bounds.len(), 1);
        assert_eq!(bounds[0].id, 1);
        assert_eq!((bounds[0].width, bounds[0].height), (100, 200));
    }

    #[test]
    fn layout_menata_child_secara_vertikal() {
        let mut h = ViewHierarchy::new();
        let root = h
            .add_view(
                View::new(1, "LinearLayout", ViewKind::ViewGroup)
                    .with_layout_params(LayoutParams::match_parent()),
                None,
            )
            .unwrap();
        h.add_view(
            View::new(2, "TextView", ViewKind::TextView)
                .with_layout_params(LayoutParams::wrap_content()),
            Some(root),
        )
        .unwrap();
        h.add_view(
            View::new(3, "TextView", ViewKind::TextView)
                .with_layout_params(LayoutParams::wrap_content()),
            Some(root),
        )
        .unwrap();

        let bounds = h.layout(100, 200);
        assert_eq!(bounds.len(), 3);
        let child1 = bounds.iter().find(|b| b.id == 2).unwrap();
        let child2 = bounds.iter().find(|b| b.id == 3).unwrap();
        assert_eq!(child1.y, 0);
        assert_eq!(child2.y, 48);
        assert_eq!(child1.width, 100);
    }

    #[test]
    fn layout_hierarki_kosong_menghasilkan_kosong() {
        let h = ViewHierarchy::new();
        assert!(h.layout(100, 100).is_empty());
    }

    #[test]
    fn view_error_tampilkan() {
        let e = ViewError::NotFound("id-5".into());
        assert_eq!(e.to_string(), "view tidak ditemukan: id-5");
        let e2 = ViewError::DuplicateId(7);
        assert_eq!(e2.to_string(), "id view duplikat: 7");
        let e3 = ViewError::NotViewGroup("TextView".into());
        assert_eq!(
            e3.to_string(),
            "view bukan ViewGroup, tidak dapat menambah child: TextView"
        );
    }
}
