//! Edit-PDF overlay: build a vector/text/image overlay PDF from the editor
//! document and stamp it with `qpdf --overlay`. Original page content is never
//! rasterized; the source file is never overwritten.

use crate::error::AppError;
use crate::models::{JobHandle, PageGroup};
use crate::pdf_engine::edit_links::{
    apply_link_annots_for_pages, dest_has_supported_links, dest_ranges_to_rewrite,
    expected_dest_has_annots, list_link_annots, unsafe_uri_error, uri_is_allowed, LinkAction,
    SessionLink, MAX_LINKS,
};
use crate::pdf_engine::validate_output::{
    catalog_flags_from_doc, content_digest, validate_staged_pdf, ContentDigest, OutputSnapshot,
    PageSnapshot,
};
use crate::pdf_engine::{crop, edit_image, qpdf};
use crate::utils::process::{run_qpdf, run_tracked};
use crate::utils::safe_output;
use crate::utils::temp;
use lopdf::{Document, Object};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::Manager;
use ttf_parser::{Face, GlyphId};

const MAX_OBJECTS: usize = 500;
const MAX_INK_POINTS: usize = 8_000;
const MAX_TEXT_CHARS: usize = 8_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditDocumentIn {
    pub version: u32,
    pub objects: Vec<EditObjectIn>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfRectIn {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PointIn {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EditObjectIn {
    Rect {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        fill: Option<String>,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Ellipse {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        fill: Option<String>,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Triangle {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        fill: Option<String>,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Star {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        fill: Option<String>,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    #[serde(rename = "roundRect")]
    RoundRect {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        fill: Option<String>,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Hexagon {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        fill: Option<String>,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Bubble {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        fill: Option<String>,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Arrow {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        fill: Option<String>,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Text {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        content: String,
        #[serde(rename = "fontSize")]
        font_size: f64,
        color: Option<String>,
        align: Option<String>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Image {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        path: String,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
        #[serde(default, rename = "keepAspect")]
        keep_aspect: Option<bool>,
    },
    Line {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Ink {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        points: Vec<PointIn>,
        stroke: Option<String>,
        #[serde(rename = "strokeWidth")]
        stroke_width: Option<f64>,
        opacity: Option<f64>,
        #[serde(default, rename = "objectRotate")]
        object_rotate: f64,
    },
    Link {
        #[serde(rename = "pageIndex")]
        page_index: u32,
        rect: PdfRectIn,
        action: LinkActionIn,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LinkActionIn {
    Uri {
        uri: String,
    },
    Goto {
        #[serde(rename = "destPageIndex")]
        dest_page_index: u32,
    },
}

impl EditObjectIn {
    fn page_index(&self) -> u32 {
        match self {
            Self::Rect { page_index, .. }
            | Self::Ellipse { page_index, .. }
            | Self::Triangle { page_index, .. }
            | Self::Star { page_index, .. }
            | Self::RoundRect { page_index, .. }
            | Self::Hexagon { page_index, .. }
            | Self::Bubble { page_index, .. }
            | Self::Arrow { page_index, .. }
            | Self::Text { page_index, .. }
            | Self::Image { page_index, .. }
            | Self::Line { page_index, .. }
            | Self::Ink { page_index, .. }
            | Self::Link { page_index, .. } => *page_index,
        }
    }
    fn opacity(&self) -> f64 {
        let o = match self {
            Self::Link { .. } => return 1.0,
            Self::Rect { opacity, .. }
            | Self::Ellipse { opacity, .. }
            | Self::Triangle { opacity, .. }
            | Self::Star { opacity, .. }
            | Self::RoundRect { opacity, .. }
            | Self::Hexagon { opacity, .. }
            | Self::Bubble { opacity, .. }
            | Self::Arrow { opacity, .. }
            | Self::Text { opacity, .. }
            | Self::Image { opacity, .. }
            | Self::Line { opacity, .. }
            | Self::Ink { opacity, .. } => *opacity,
        };
        o.unwrap_or(1.0).clamp(0.05, 1.0)
    }

    fn object_rotate(&self) -> f64 {
        match self {
            Self::Link { .. } => 0.0,
            Self::Rect { object_rotate, .. }
            | Self::Ellipse { object_rotate, .. }
            | Self::Triangle { object_rotate, .. }
            | Self::Star { object_rotate, .. }
            | Self::RoundRect { object_rotate, .. }
            | Self::Hexagon { object_rotate, .. }
            | Self::Bubble { object_rotate, .. }
            | Self::Arrow { object_rotate, .. }
            | Self::Text { object_rotate, .. }
            | Self::Image { object_rotate, .. }
            | Self::Line { object_rotate, .. }
            | Self::Ink { object_rotate, .. } => *object_rotate,
        }
    }

    fn overlay_aabb(&self, vis: [f64; 4], page_rot: i64) -> (f64, f64, f64, f64) {
        match self {
            Self::Line { x1, y1, x2, y2, .. } => {
                let (ax, ay) = pdf_point_to_overlay(*x1, *y1, vis, page_rot);
                let (bx, by) = pdf_point_to_overlay(*x2, *y2, vis, page_rot);
                let x = ax.min(bx);
                let y = ay.min(by);
                (x, y, (ax - bx).abs().max(0.5), (ay - by).abs().max(0.5))
            }
            Self::Ink { points, .. } => {
                if points.is_empty() {
                    return (0.0, 0.0, 1.0, 1.0);
                }
                let mapped: Vec<(f64, f64)> = points
                    .iter()
                    .map(|p| pdf_point_to_overlay(p.x, p.y, vis, page_rot))
                    .collect();
                let min_x = mapped.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
                let max_x = mapped.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
                let min_y = mapped.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
                let max_y = mapped.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
                (
                    min_x,
                    min_y,
                    (max_x - min_x).max(0.5),
                    (max_y - min_y).max(0.5),
                )
            }
            Self::Rect { rect, .. }
            | Self::Ellipse { rect, .. }
            | Self::Triangle { rect, .. }
            | Self::Star { rect, .. }
            | Self::RoundRect { rect, .. }
            | Self::Hexagon { rect, .. }
            | Self::Bubble { rect, .. }
            | Self::Arrow { rect, .. }
            | Self::Text { rect, .. }
            | Self::Image { rect, .. }
            | Self::Link { rect, .. } => pdf_rect_to_overlay(rect, vis, page_rot),
        }
    }
}

fn push_object_rotate(content: &mut String, deg_cw: f64, aabb: (f64, f64, f64, f64)) -> bool {
    if deg_cw.abs() < 0.05 {
        return false;
    }
    let (x, y, w, h) = aabb;
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let th = -deg_cw.to_radians();
    let (c, s) = (th.cos(), th.sin());
    content.push_str(&format!(
        "q\n1 0 0 1 {cx:.2} {cy:.2} cm\n{c:.5} {s:.5} {:.5} {c:.5} 0 0 cm\n1 0 0 1 {:.2} {:.2} cm\n",
        -s, -cx, -cy
    ));
    true
}

struct PdfBuilder {
    buf: Vec<u8>,
    offs: Vec<usize>,
}

impl PdfBuilder {
    fn new() -> Self {
        Self {
            buf: b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n".to_vec(),
            offs: vec![0],
        }
    }
    fn begin(&mut self) -> usize {
        self.offs.push(self.buf.len());
        self.offs.len() - 1
    }
    fn s(&mut self, s: &str) {
        self.buf.extend_from_slice(s.as_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    fn finish(mut self, root: usize) -> Vec<u8> {
        let xref = self.buf.len();
        let n = self.offs.len();
        self.s(&format!("xref\n0 {n}\n0000000000 65535 f \n"));
        for i in 1..n {
            self.s(&format!("{:010} 00000 n \n", self.offs[i]));
        }
        self.s(&format!(
            "trailer\n<< /Size {n} /Root {root} 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        ));
        self.buf
    }
}

/// Map unrotated page-box-relative point into displayed overlay space (BL origin).
pub fn unrotated_to_display(rx: f64, ry: f64, box_w: f64, box_h: f64, rotate: i64) -> (f64, f64) {
    match ((rotate % 360) + 360) % 360 {
        90 => (ry, box_w - rx),
        180 => (box_w - rx, box_h - ry),
        270 => (box_h - ry, rx),
        _ => (rx, ry),
    }
}

pub fn pdf_rect_to_overlay(rect: &PdfRectIn, vis: [f64; 4], rotate: i64) -> (f64, f64, f64, f64) {
    let bw = vis[2] - vis[0];
    let bh = vis[3] - vis[1];
    let corners = [
        (rect.x, rect.y),
        (rect.x + rect.w, rect.y),
        (rect.x + rect.w, rect.y + rect.h),
        (rect.x, rect.y + rect.h),
    ];
    let mapped: Vec<(f64, f64)> = corners
        .iter()
        .map(|(x, y)| unrotated_to_display(x - vis[0], y - vis[1], bw, bh, rotate))
        .collect();
    let min_x = mapped.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = mapped.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = mapped.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = mapped.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    (
        min_x,
        min_y,
        (max_x - min_x).max(0.5),
        (max_y - min_y).max(0.5),
    )
}

pub fn pdf_point_to_overlay(x: f64, y: f64, vis: [f64; 4], rotate: i64) -> (f64, f64) {
    let bw = vis[2] - vis[0];
    let bh = vis[3] - vis[1];
    unrotated_to_display(x - vis[0], y - vis[1], bw, bh, rotate)
}

fn parse_hex(color: Option<&str>, fallback: (f64, f64, f64)) -> (f64, f64, f64) {
    let Some(s) = color else {
        return fallback;
    };
    let t = s.trim().trim_start_matches('#');
    if t.len() == 6 {
        if let Ok(n) = u32::from_str_radix(t, 16) {
            return (
                ((n >> 16) & 255) as f64 / 255.0,
                ((n >> 8) & 255) as f64 / 255.0,
                (n & 255) as f64 / 255.0,
            );
        }
    }
    fallback
}

fn fill_is_none(fill: Option<&str>) -> bool {
    match fill {
        None => true,
        Some(s) => {
            let t = s.trim().to_ascii_lowercase();
            t.is_empty() || t == "none" || t == "transparent"
        }
    }
}

fn paint_path(
    content: &mut String,
    fill: Option<&str>,
    stroke: Option<&str>,
    stroke_width: Option<f64>,
    path: &str,
) {
    let fill_none = fill_is_none(fill);
    let (sr, sg, sb) = parse_hex(stroke, (0.067, 0.094, 0.153));
    let sw = stroke_width.unwrap_or(1.5).clamp(0.2, 24.0);
    if fill_none {
        content.push_str(&format!("{sr:.3} {sg:.3} {sb:.3} RG\n{sw:.2} w\n{path}S\n"));
    } else {
        let (fr, fg, fb) = parse_hex(fill, (0.067, 0.094, 0.153));
        content.push_str(&format!(
            "{fr:.3} {fg:.3} {fb:.3} rg\n{sr:.3} {sg:.3} {sb:.3} RG\n{sw:.2} w\n{path}B\n"
        ));
    }
}

fn ellipse_path(x: f64, y: f64, w: f64, h: f64) -> String {
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let rx = (w / 2.0).max(0.25);
    let ry = (h / 2.0).max(0.25);
    let k = 0.552_284_749_8;
    let ox = rx * k;
    let oy = ry * k;
    format!(
        "{:.2} {:.2} m\n\
         {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n\
         {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n\
         {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n\
         {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\nh\n",
        cx + rx,
        cy,
        cx + rx,
        cy + oy,
        cx + ox,
        cy + ry,
        cx,
        cy + ry,
        cx - ox,
        cy + ry,
        cx - rx,
        cy + oy,
        cx - rx,
        cy,
        cx - rx,
        cy - oy,
        cx - ox,
        cy - ry,
        cx,
        cy - ry,
        cx + ox,
        cy - ry,
        cx + rx,
        cy - oy,
        cx + rx,
        cy,
    )
}

fn polygon_path(pts: &[(f64, f64)]) -> String {
    let mut s = String::new();
    for (i, (px, py)) in pts.iter().enumerate() {
        if i == 0 {
            s.push_str(&format!("{px:.2} {py:.2} m\n"));
        } else {
            s.push_str(&format!("{px:.2} {py:.2} l\n"));
        }
    }
    s.push_str("h\n");
    s
}

fn triangle_pts(x: f64, y: f64, w: f64, h: f64) -> [(f64, f64); 3] {
    [(x + w / 2.0, y + h), (x, y), (x + w, y)]
}

fn round_rect_path(x: f64, y: f64, w: f64, h: f64) -> String {
    let r = (w.min(h) * 0.18).min(w / 2.0).min(h / 2.0).max(0.5);
    let k = 0.552_284_749_8 * r;
    format!(
        "{:.2} {:.2} m\n{:.2} {:.2} l\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n\
         {:.2} {:.2} l\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n\
         {:.2} {:.2} l\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n\
         {:.2} {:.2} l\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\nh\n",
        x + r,
        y,
        x + w - r,
        y,
        x + w - r + k,
        y,
        x + w,
        y + r - k,
        x + w,
        y + r,
        x + w,
        y + h - r,
        x + w,
        y + h - r + k,
        x + w - r + k,
        y + h,
        x + w - r,
        y + h,
        x + r,
        y + h,
        x + r - k,
        y + h,
        x,
        y + h - r + k,
        x,
        y + h - r,
        x,
        y + r,
        x,
        y + r - k,
        x + r - k,
        y,
        x + r,
        y,
    )
}

fn hexagon_pts(x: f64, y: f64, w: f64, h: f64) -> Vec<(f64, f64)> {
    (0..6)
        .map(|i| {
            let a = std::f64::consts::PI / 3.0 * i as f64 + std::f64::consts::PI / 6.0;
            (
                x + w / 2.0 + (w / 2.0) * a.cos(),
                y + h / 2.0 + (h / 2.0) * a.sin(),
            )
        })
        .collect()
}

fn arrow_pts(x: f64, y: f64, w: f64, h: f64) -> Vec<(f64, f64)> {
    let nx = x + w * 0.55;
    let mid = y + h / 2.0;
    vec![
        (x, y + h * 0.72),
        (nx, y + h * 0.72),
        (nx, y + h),
        (x + w, mid),
        (nx, y),
        (nx, y + h * 0.28),
        (x, y + h * 0.28),
    ]
}

fn bubble_path(x: f64, y: f64, w: f64, h: f64) -> String {
    let tail = (h * 0.22).min(28.0);
    let by = y + tail;
    let bh = (h - tail).max(8.0);
    let r = (w.min(bh) * 0.16).max(0.5);
    let k = 0.552_284_749_8 * r;
    let t1x = x + w * 0.18;
    let t2x = x + w * 0.08;
    let t3x = x + w * 0.36;
    format!(
        "{:.2} {:.2} m\n{:.2} {:.2} l\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n\
         {:.2} {:.2} l\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n\
         {:.2} {:.2} l\n{:.2} {:.2} l\n{:.2} {:.2} l\n{:.2} {:.2} l\n\
         {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\n\
         {:.2} {:.2} l\n{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c\nh\n",
        x + r,
        by + bh,
        x + w - r,
        by + bh,
        x + w - r + k,
        by + bh,
        x + w,
        by + bh - r + k,
        x + w,
        by + bh - r,
        x + w,
        by + r,
        x + w,
        by + r - k,
        x + w - r + k,
        by,
        x + w - r,
        by,
        t3x,
        by,
        t2x,
        y,
        t1x,
        by,
        x + r,
        by,
        x + r - k,
        by,
        x,
        by + r - k,
        x,
        by + r,
        x,
        by + bh - r,
        x,
        by + bh - r + k,
        x + r - k,
        by + bh,
        x + r,
        by + bh,
    )
}

fn star_pts(x: f64, y: f64, w: f64, h: f64) -> Vec<(f64, f64)> {
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let rx = w / 2.0;
    let ry = h / 2.0;
    let inner = 0.382;
    let mut pts = Vec::with_capacity(10);
    for i in 0..10 {
        let a = std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0;
        let r = if i % 2 == 0 { 1.0 } else { inner };
        pts.push((cx + rx * r * a.cos(), cy + ry * r * a.sin()));
    }
    pts
}

struct FontInfo {
    data: Vec<u8>,
    units_per_em: f64,
    bbox: [i16; 4],
    ascent: i16,
    descent: i16,
}

impl FontInfo {
    fn parse(data: Vec<u8>) -> Result<Self, AppError> {
        let face = Face::parse(&data, 0).map_err(|_| {
            AppError::new(
                "BAD_FONT",
                "Editor font unreadable",
                "The bundled font is damaged.",
            )
        })?;
        let bb = face.global_bounding_box();
        Ok(Self {
            units_per_em: face.units_per_em() as f64,
            bbox: [bb.x_min, bb.y_min, bb.x_max, bb.y_max],
            ascent: face.ascender(),
            descent: face.descender(),
            data,
        })
    }
    fn face(&self) -> Face<'_> {
        Face::parse(&self.data, 0).expect("font already parsed")
    }
    fn gid(&self, ch: char) -> u16 {
        self.face().glyph_index(ch).map(|g| g.0).unwrap_or(0)
    }
    fn width(&self, gid: u16) -> f64 {
        let adv = self.face().glyph_hor_advance(GlyphId(gid)).unwrap_or(0) as f64;
        adv / self.units_per_em * 1000.0
    }
    fn scale(&self) -> f64 {
        1000.0 / self.units_per_em
    }
}

fn wrap_text(font: &FontInfo, text: &str, font_size: f64, max_w: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        if para.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_w = 0.0;
        for ch in para.chars() {
            let cw = font.width(font.gid(ch)) * font_size / 1000.0;
            if !cur.is_empty() && cur_w + cw > max_w && max_w > 8.0 {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0.0;
            }
            cur.push(ch);
            cur_w += cw;
        }
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn line_hex_and_width(font: &FontInfo, text: &str) -> (String, f64) {
    let mut hex = String::new();
    let mut w = 0.0;
    for ch in text.chars() {
        let gid = font.gid(ch);
        hex.push_str(&format!("{gid:04X}"));
        w += font.width(gid);
    }
    (hex, w)
}

fn find_font_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppError> {
    if let Ok(res) = app.path().resource_dir() {
        for p in [
            res.join("fonts").join("NotoSans-Regular.ttf"),
            res.join("resources")
                .join("fonts")
                .join("NotoSans-Regular.ttf"),
        ] {
            if p.exists() {
                return Ok(p);
            }
        }
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("fonts")
        .join("NotoSans-Regular.ttf");
    if dev.exists() {
        return Ok(dev);
    }
    Err(AppError::new(
        "NO_FONT",
        "Editor font missing",
        "The bundled Noto Sans font could not be found.",
    )
    .with_suggestion("Reinstall OffPDF."))
}

fn validate_doc(doc: &EditDocumentIn) -> Result<(), AppError> {
    if doc.version != 1 {
        return Err(AppError::new(
            "BAD_EDIT",
            "Unsupported edit data",
            "This edit was created with a newer OffPDF version.",
        ));
    }
    let mut paint = 0usize;
    let mut links = 0usize;
    for o in &doc.objects {
        if matches!(o, EditObjectIn::Link { .. }) {
            links += 1;
        } else {
            paint += 1;
        }
    }
    if paint > MAX_OBJECTS {
        return Err(AppError::new(
            "TOO_MANY_OBJECTS",
            "Too many objects",
            format!("This edit has more than {MAX_OBJECTS} objects."),
        ));
    }
    if links > MAX_LINKS {
        return Err(AppError::new(
            "TOO_MANY_LINKS",
            "Too many links",
            format!("This edit has more than {MAX_LINKS} links."),
        ));
    }
    for o in &doc.objects {
        match o {
            EditObjectIn::Text { content, .. } if content.chars().count() > MAX_TEXT_CHARS => {
                return Err(AppError::new(
                    "TEXT_TOO_LONG",
                    "Text is too long",
                    "Shorten the text box and try again.",
                ));
            }
            EditObjectIn::Ink { points, .. } if points.len() > MAX_INK_POINTS => {
                return Err(AppError::new(
                    "INK_TOO_LONG",
                    "Drawing is too complex",
                    "Use a shorter stroke.",
                ));
            }
            EditObjectIn::Link {
                action: LinkActionIn::Uri { uri },
                ..
            } if !uri_is_allowed(uri) => {
                return Err(unsafe_uri_error(uri));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Per-page geometry read from the original sources (not a `--empty` rebuild).
#[derive(Debug, Clone, Copy)]
struct OverlayPageGeom {
    /// Crop ∩ Media (else Media). Overlay Media/Crop/Trim use this so qpdf
    /// `getTrimBox()` is the full visible page (no Trim⊂Media centering).
    visible: [f64; 4],
    /// Original effective page boxes. The temporary qpdf source normalizes all
    /// three boxes to `visible`; these values restore the exported page.
    media: [f64; 4],
    crop: Option<[f64; 4]>,
    /// Page-level only: qpdf overlay alignment does not inherit TrimBox.
    trim: Option<[f64; 4]>,
    rotate: i64,
    user_unit: f64,
    content_digest: ContentDigest,
}

fn boxes_near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 0.5)
}

fn pdf_rect_obj(b: [f64; 4]) -> Object {
    Object::Array(vec![
        Object::Real(b[0] as f32),
        Object::Real(b[1] as f32),
        Object::Real(b[2] as f32),
        Object::Real(b[3] as f32),
    ])
}

fn set_page_box(
    doc: &mut Document,
    page_id: lopdf::ObjectId,
    key: &[u8],
    value: Option<[f64; 4]>,
) -> Result<(), AppError> {
    let dict = doc
        .get_object_mut(page_id)
        .and_then(|o| o.as_dict_mut())
        .map_err(|e| AppError::engine_failed(format!("Could not update page boxes: {e}")))?;
    if let Some(b) = value {
        dict.set(key.to_vec(), pdf_rect_obj(b));
    } else {
        dict.remove(key);
    }
    Ok(())
}

/// Working copy whose Media/Crop/Trim boxes all equal the visible box. qpdf
/// centers a page's alignment box inside its MediaBox while composing; changing
/// Trim alone therefore moves existing content when Crop is asymmetric. Making
/// all three boxes identical keeps both the source and overlay transforms 1:1.
/// Never writes `src`.
fn write_visible_box_copy(src: &Path, dest: &Path) -> Result<(), AppError> {
    let mut doc = Document::load(src)
        .map_err(|e| AppError::engine_failed(format!("Could not read the PDF: {e}")))?;
    let page_ids: Vec<_> = doc.get_pages().values().cloned().collect();
    for id in page_ids {
        let vis = crop::visible_box(&doc, id);
        set_page_box(&mut doc, id, b"MediaBox", Some(vis))?;
        set_page_box(&mut doc, id, b"CropBox", Some(vis))?;
        set_page_box(&mut doc, id, b"TrimBox", Some(vis))?;
    }
    doc.save(dest)
        .map_err(|e| AppError::io("Could not write a temporary PDF.", e))?;
    Ok(())
}

/// Point `--overlay` at normalized working copies whenever qpdf's alignment box
/// or MediaBox differs from the visible page. Unchanged sources keep their
/// original path (identity overlay argv stays the user file).
fn remap_groups_to_visible_box(
    groups: &[PageGroup],
    work: &Path,
) -> Result<(Vec<PageGroup>, bool), AppError> {
    let mut copies: HashMap<String, String> = HashMap::new();
    let mut any = false;
    let mut n_copies = 0usize;
    let mut out = Vec::with_capacity(groups.len());
    for g in groups {
        if !copies.contains_key(&g.path) {
            let doc = Document::load(&g.path)
                .map_err(|e| AppError::engine_failed(format!("Could not read the PDF: {e}")))?;
            let needs = doc.get_pages().values().any(|&id| {
                let visible = crop::visible_box(&doc, id);
                !boxes_near(crop::align_box(&doc, id), visible)
                    || !boxes_near(crop::media_box(&doc, id), visible)
            });
            if needs {
                let dest = work.join(format!("src-{n_copies}.pdf"));
                n_copies += 1;
                write_visible_box_copy(Path::new(&g.path), &dest)?;
                copies.insert(g.path.clone(), dest.to_string_lossy().into_owned());
                any = true;
            } else {
                copies.insert(g.path.clone(), g.path.clone());
            }
        }
        out.push(PageGroup {
            path: copies.get(&g.path).expect("path just inserted").clone(),
            pages: g.pages.clone(),
        });
    }
    Ok((out, any))
}

/// Restore the original page boxes after qpdf composed against normalized
/// working pages. Missing CropBox/TrimBox entries remain missing.
fn restore_dest_page_boxes(dest: &Path, geoms: &[OverlayPageGeom]) -> Result<(), AppError> {
    let mut doc = Document::load(dest)
        .map_err(|e| AppError::engine_failed(format!("Could not read the PDF: {e}")))?;
    let page_map = doc.get_pages();
    for (i, geom) in geoms.iter().enumerate() {
        let p = (i as u32) + 1;
        let id = *page_map
            .get(&p)
            .ok_or_else(|| AppError::engine_failed(format!("Page {p} is not in the saved PDF.")))?;
        set_page_box(&mut doc, id, b"MediaBox", Some(geom.media))?;
        set_page_box(&mut doc, id, b"CropBox", geom.crop)?;
        set_page_box(&mut doc, id, b"TrimBox", geom.trim)?;
    }
    let side = dest.with_file_name(format!(
        "{}.boxes",
        dest.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    doc.save(&side)
        .map_err(|e| AppError::io("Could not write the output file.", e))?;
    safe_output::replace_file(&side, dest)
}

fn fmt_pdf_box(b: [f64; 4]) -> String {
    format!("{:.2} {:.2} {:.2} {:.2}", b[0], b[1], b[2], b[3])
}

/// Center a raster of `iw×ih` inside overlay AABB `(x,y,w,h)` (SVG `meet`).
pub(crate) fn image_meet_blit(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    iw: f64,
    ih: f64,
) -> (f64, f64, f64, f64) {
    let iw = iw.max(1.0);
    let ih = ih.max(1.0);
    let s = (w / iw).min(h / ih);
    let dw = iw * s;
    let dh = ih * s;
    (x + (w - dw) / 2.0, y + (h - dh) / 2.0, dw, dh)
}

/// Overlay page box after applying the same zero-origin rotation window used by
/// `pdf_rect_to_overlay` to absolute source coordinates. A non-zero visible
/// origin must remain in this transformed box or qpdf applies a second offset.
fn overlay_page_box(vis: [f64; 4], rotate: i64) -> [f64; 4] {
    let w = vis[2] - vis[0];
    let h = vis[3] - vis[1];
    match ((rotate % 360) + 360) % 360 {
        90 => [vis[1], w - vis[2], vis[3], w - vis[0]],
        180 => [w - vis[2], h - vis[3], w - vis[0], h - vis[1]],
        270 => [h - vis[3], vis[0], h - vis[1], vis[2]],
        _ => vis,
    }
}

/// Expand a qpdf-style page spec into 1-based numbers, preserving order.
/// Unlike `parse_pages`, this does not sort or dedupe.
fn expand_page_spec(spec: &str, n: u32) -> Result<Vec<u32>, AppError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") || trimmed == "1-z" {
        return Ok((1..=n).collect());
    }
    let parse_tok = |raw: &str| -> Result<u32, AppError> {
        let t = raw.trim();
        if t.eq_ignore_ascii_case("z") {
            return Ok(n);
        }
        t.parse::<u32>().map_err(|_| {
            AppError::new(
                "INVALID_PAGES",
                "Invalid page selection",
                format!("\"{spec}\" is not a valid page selection."),
            )
            .with_suggestion("Use page numbers and ranges like \"1,3,5-8\".")
        })
    };
    let mut out: Vec<u32> = Vec::new();
    for raw in trimmed.split(',') {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            let lo = parse_tok(a)?;
            let hi = parse_tok(b)?;
            if lo <= hi {
                for p in lo..=hi {
                    out.push(p);
                }
            } else {
                for p in (hi..=lo).rev() {
                    out.push(p);
                }
            }
        } else {
            out.push(parse_tok(part)?);
        }
    }
    if out.is_empty() {
        return Err(AppError::new(
            "INVALID_PAGES",
            "Invalid page selection",
            format!("\"{spec}\" is not a valid page selection."),
        ));
    }
    if out.iter().any(|p| *p < 1 || *p > n) {
        return Err(AppError::new(
            "INVALID_PAGES",
            "Page out of range",
            format!("This PDF has {n} page{}.", if n == 1 { "" } else { "s" }),
        ));
    }
    Ok(out)
}

fn collect_source_pages(
    groups: &[PageGroup],
) -> Result<(Vec<OverlayPageGeom>, Vec<u32>), AppError> {
    let mut docs: HashMap<String, Document> = HashMap::new();
    let mut geoms = Vec::new();
    let mut counts = Vec::with_capacity(groups.len());
    for g in groups {
        if !docs.contains_key(&g.path) {
            let doc = Document::load(&g.path)
                .map_err(|e| AppError::engine_failed(format!("Could not read the PDF: {e}")))?;
            docs.insert(g.path.clone(), doc);
        }
        let doc = docs.get(&g.path).expect("document just loaded");
        let page_map = doc.get_pages();
        let n = page_map.len() as u32;
        counts.push(n);
        for p in expand_page_spec(&g.pages, n)? {
            let id = *page_map.get(&p).ok_or_else(|| {
                AppError::new(
                    "INVALID_PAGES",
                    "Page out of range",
                    format!("Page {p} is not in this PDF."),
                )
            })?;
            let content = doc.get_page_content(id).map_err(|e| {
                AppError::engine_failed(format!("Could not read page content: {e}"))
            })?;
            geoms.push(OverlayPageGeom {
                visible: crop::visible_box(doc, id),
                media: crop::media_box(doc, id),
                crop: crop::crop_box(doc, id),
                trim: crop::page_trim_box(doc, id),
                rotate: crop::page_rotation(doc, id),
                user_unit: crop::page_user_unit(doc, id),
                content_digest: content_digest(&content),
            });
        }
    }
    Ok((geoms, counts))
}

/// qpdf argv for Edit PDF. Never uses `--empty`: the first source is the
/// primary input so bookmarks, Info/XMP, and AcroForm survive when possible.
pub(crate) fn build_edit_overlay_args(
    groups: &[PageGroup],
    page_counts: &[u32],
    overlay: &str,
    dest: &str,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }
    if groups.len() != page_counts.len() {
        return Err(AppError::new(
            "BAD_EDIT",
            "Could not save",
            "The page list does not match the open documents.",
        ));
    }
    let primary = groups[0].path.clone();
    let identity = groups.len() == 1 && super::spec_is_full_range(&groups[0].pages, page_counts[0]);
    let mut args = vec![primary];
    if !identity {
        args.push("--pages".into());
        args.push(".".into());
        args.push(groups[0].pages.clone());
        for g in &groups[1..] {
            args.push(g.path.clone());
            args.push(g.pages.clone());
        }
        // `--pages` must be closed before `--overlay` or qpdf treats the flag
        // as another filename.
        args.push("--".into());
    }
    args.push("--overlay".into());
    args.push(overlay.to_string());
    args.push("--".into());
    args.push(dest.to_string());
    Ok(args)
}

/// When hydrate failed for a source, a session `kind: "link"` on that file's
/// dest pages must return the same list `AppError` instead of rewriting.
fn reject_links_on_unlistable_sources(
    groups: &[PageGroup],
    counts: &[u32],
    links: &[SessionLink],
) -> Result<(), AppError> {
    let mut dest_base = 0u32;
    for (g, &n) in groups.iter().zip(counts.iter()) {
        let end = dest_base.saturating_add(n);
        let has = links
            .iter()
            .any(|l| l.page_index >= dest_base && l.page_index < end);
        dest_base = end;
        if !has {
            continue;
        }
        list_link_annots(Path::new(&g.path))?;
    }
    Ok(())
}

fn session_links_from_doc(doc: &EditDocumentIn) -> Vec<SessionLink> {
    doc.objects
        .iter()
        .filter_map(|o| match o {
            EditObjectIn::Link {
                page_index,
                rect,
                action,
            } => Some(SessionLink {
                page_index: *page_index,
                rect: [rect.x, rect.y, rect.w, rect.h],
                action: match action {
                    LinkActionIn::Uri { uri } => LinkAction::Uri { uri: uri.clone() },
                    LinkActionIn::Goto { dest_page_index } => LinkAction::GoTo {
                        dest_page_index: *dest_page_index,
                    },
                },
            }),
            _ => None,
        })
        .collect()
}

/// Link-only save: assemble to dest without `--empty` and without overlay paint.
fn assemble_to_tmp<F>(
    groups: &[PageGroup],
    page_counts: &[u32],
    tmp: &Path,
    tmp_str: &str,
    run: &mut F,
) -> Result<(), AppError>
where
    F: FnMut(&[String]) -> Result<(), AppError>,
{
    if groups.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }
    let identity = groups.len() == 1 && super::spec_is_full_range(&groups[0].pages, page_counts[0]);
    if identity {
        std::fs::copy(&groups[0].path, tmp)
            .map_err(|e| AppError::io("Could not prepare the output file.", e))?;
        return Ok(());
    }
    let mut args = vec![
        groups[0].path.clone(),
        "--pages".into(),
        ".".into(),
        groups[0].pages.clone(),
    ];
    for g in &groups[1..] {
        args.push(g.path.clone());
        args.push(g.pages.clone());
    }
    args.push("--".into());
    args.push(tmp_str.to_string());
    run(&args)
}

/// Build the overlay and run qpdf via `run`. Used by tests (system/`"qpdf"`).
pub(crate) fn export_edit_pdf_with_runner<F>(
    groups: &[PageGroup],
    output: &str,
    document: &EditDocumentIn,
    font_path: &Path,
    work: &Path,
    unique: &str,
    cancel: Option<&AtomicBool>,
    run: F,
) -> Result<Vec<String>, AppError>
where
    F: FnMut(&[String]) -> Result<(), AppError>,
{
    let exe = qpdf::resolve_qpdf_standalone();
    export_edit_pdf_with_check_exe(
        groups,
        output,
        document,
        font_path,
        work,
        unique,
        cancel,
        &exe,
        None,
        &[],
        run,
    )
    .map(|(paths, _warnings)| paths)
}

/// Same as [`export_edit_pdf_with_runner`], with an explicit `qpdf --check` binary.
fn export_edit_pdf_with_check_exe<F>(
    groups: &[PageGroup],
    output: &str,
    document: &EditDocumentIn,
    font_path: &Path,
    work: &Path,
    unique: &str,
    cancel: Option<&AtomicBool>,
    qpdf_check: &Path,
    handle: Option<&Arc<JobHandle>>,
    incomplete_source_paths: &[String],
    mut run: F,
) -> Result<(Vec<String>, Vec<String>), AppError>
where
    F: FnMut(&[String]) -> Result<(), AppError>,
{
    validate_doc(document)?;
    let dest = Path::new(output);
    for g in groups {
        if safe_output::same_file_identity(Path::new(&g.path), dest) {
            return Err(AppError::new(
                "OVERWRITE",
                "Choose a new file name",
                "OffPDF never overwrites the original PDF.",
            )
            .with_suggestion("Pick a different name or folder."));
        }
    }
    let tmp = safe_output::sibling_temp_path(dest, unique)?;
    let tmp_str = tmp.to_string_lossy().to_string();
    let overlay = work.join("overlay.pdf");
    let overlay_str = overlay.to_string_lossy().to_string();
    let mut gate_passed = false;
    let result = (|| -> Result<(Vec<String>, Vec<String>), AppError> {
        let (geoms, counts) = collect_source_pages(groups)?;
        if geoms.is_empty() {
            return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
        }
        let font_bytes = std::fs::read(font_path)
            .map_err(|e| AppError::io("Could not read the editor font.", e))?;
        let font = FontInfo::parse(font_bytes)?;
        let links = session_links_from_doc(document);
        let has_paint = document
            .objects
            .iter()
            .any(|o| !matches!(o, EditObjectIn::Link { .. }));
        if has_paint {
            write_overlay_pdf(&overlay_str, &geoms, document, &font, cancel)?;
            let (mapped, restore_boxes) = remap_groups_to_visible_box(groups, work)?;
            let args = build_edit_overlay_args(&mapped, &counts, &overlay_str, &tmp_str)?;
            run(&args)?;
            if restore_boxes {
                restore_dest_page_boxes(&tmp, &geoms)?;
                // lopdf xref can look damaged; qpdf rewrite before the atomic replace.
                let cleaned = work.join("dest-boxes.pdf");
                let cleaned_str = cleaned.to_string_lossy().to_string();
                run(&[tmp_str.clone(), cleaned_str.clone()])?;
                safe_output::replace_file(&cleaned, &tmp)?;
            }
        } else {
            assemble_to_tmp(groups, &counts, &tmp, &tmp_str, &mut run)?;
        }
        let expected_annots = expected_dest_has_annots(&tmp, &links)?;
        if !incomplete_source_paths.is_empty() && !links.is_empty() {
            reject_links_on_unlistable_sources(groups, &counts, &links)?;
        }
        let group_pairs: Vec<(&str, u32)> = groups
            .iter()
            .zip(counts.iter())
            .map(|(g, &n)| (g.path.as_str(), n))
            .collect();
        let incomplete_refs: Vec<&str> =
            incomplete_source_paths.iter().map(String::as_str).collect();
        let dest_pages: Vec<u32> = dest_ranges_to_rewrite(&group_pairs, &incomplete_refs)
            .into_iter()
            .flatten()
            .collect();
        // Skip dest_has load when no complete source remains (e.g. 400 MiB
        // unlistable file). Complete empty still deletes when dest has links (L7).
        let rewrite_links = !dest_pages.is_empty()
            && (!links.is_empty() || dest_has_supported_links(&tmp)?);
        if rewrite_links {
            apply_link_annots_for_pages(&tmp, &links, &dest_pages)?;
            let cleaned = work.join("dest-links.pdf");
            let cleaned_str = cleaned.to_string_lossy().to_string();
            run(&[tmp_str.clone(), cleaned_str.clone()])?;
            safe_output::replace_file(&cleaned, &tmp)?;
        }
        let mut snapshot = output_snapshot_from_source(&geoms, Path::new(&groups[0].path))?;
        snapshot.catalog.annots = expected_annots;
        let vr = validate_staged_pdf(&tmp, &snapshot, cancel, |args| {
            run_qpdf_check_argv(qpdf_check, args, handle)
        })?;
        gate_passed = true;
        safe_output::replace_file(&tmp, dest)?;
        Ok((vec![output.to_string()], vr.warnings))
    })();
    // Keep tmp only if replace_file failed after a passed gate (Windows recover).
    // Spawn/validate errors (and leftover success tmp) delete the sibling.
    if !(gate_passed && result.is_err()) && tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn output_snapshot_from_source(
    geoms: &[OverlayPageGeom],
    primary: &Path,
) -> Result<OutputSnapshot, AppError> {
    let doc = Document::load(primary)
        .map_err(|e| AppError::engine_failed(format!("Could not read the PDF: {e}")))?;
    Ok(OutputSnapshot {
        pages: geoms
            .iter()
            .map(|g| PageSnapshot {
                media_box: g.media,
                crop_box: g.crop,
                trim_box: g.trim,
                rotate: g.rotate,
                user_unit: g.user_unit,
                content_digest: g.content_digest,
            })
            .collect(),
        catalog: catalog_flags_from_doc(&doc),
    })
}

fn run_qpdf_check_argv(
    exe: &Path,
    args: &[String],
    handle: Option<&Arc<JobHandle>>,
) -> Result<(i32, String), AppError> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    if let Some(h) = handle {
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::piped());
        let (status, stderr) = run_tracked(h, cmd)?;
        return Ok((status.and_then(|s| s.code()).unwrap_or(1), stderr));
    }
    let output = cmd
        .output()
        .map_err(|e| AppError::io("qpdf --check failed to start", e))?;
    Ok((
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

pub fn edit_pdf_overlays(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
    document: &EditDocumentIn,
    incomplete_source_paths: &[String],
) -> Result<(Vec<String>, Vec<String>), AppError> {
    if groups.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }
    for g in groups {
        super::require_input(&g.path)?;
        if safe_output::same_file_identity(Path::new(&g.path), Path::new(output)) {
            return Err(AppError::new(
                "OVERWRITE",
                "Choose a new file name",
                "OffPDF never overwrites the original PDF.",
            )
            .with_suggestion("Pick a different name or folder."));
        }
    }
    super::ensure_output_dir(output)?;
    validate_doc(document)?;

    let work = temp::root(app)?.join("work").join(job_id);
    std::fs::create_dir_all(&work)
        .map_err(|e| AppError::io("Could not create a temp directory.", e))?;
    let font_path = find_font_path(app)?;

    let result = (|| -> Result<(Vec<String>, Vec<String>), AppError> {
        if handle.is_cancelled() {
            return Err(AppError::cancelled());
        }
        let qpdf_exe = qpdf::resolve_qpdf(app);
        export_edit_pdf_with_check_exe(
            groups,
            output,
            document,
            &font_path,
            &work,
            job_id,
            Some(&handle.cancelled),
            &qpdf_exe,
            Some(handle),
            incomplete_source_paths,
            |args| run_qpdf(app, handle, job_id, args, "Saving", None),
        )
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}

fn collect_unique_rasters(
    objects: &[EditObjectIn],
    mut check: impl FnMut() -> Result<(), AppError>,
) -> Result<(Vec<edit_image::Raster>, Vec<Option<usize>>), AppError> {
    let mut rasters: Vec<edit_image::Raster> = Vec::new();
    let mut image_for_obj: Vec<Option<usize>> = vec![None; objects.len()];
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut unique_bytes = 0u64;
    let mut unique_pixels = 0u64;
    for (i, o) in objects.iter().enumerate() {
        if let EditObjectIn::Image { path, .. } = o {
            check()?;
            let key = edit_image::dedupe_key(path);
            if let Some(&idx) = seen.get(&key) {
                image_for_obj[i] = Some(idx);
                continue;
            }
            let rast = edit_image::load_image_rgb(path)?;
            unique_bytes += rast.source_len;
            unique_pixels += rast.w as u64 * rast.h as u64;
            edit_image::check_doc_budget(unique_bytes, unique_pixels)?;
            seen.insert(key, rasters.len());
            image_for_obj[i] = Some(rasters.len());
            rasters.push(rast);
            check()?;
        }
    }
    Ok((rasters, image_for_obj))
}

fn write_overlay_pdf(
    overlay_path: &str,
    geoms: &[OverlayPageGeom],
    document: &EditDocumentIn,
    font: &FontInfo,
    cancel: Option<&AtomicBool>,
) -> Result<(), AppError> {
    let n = geoms.len();
    if n == 0 {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }

    let mut used: Vec<(u16, char)> = Vec::new();
    for o in &document.objects {
        if let EditObjectIn::Text { content, .. } = o {
            for ch in content.chars() {
                if ch != '\n' && ch != '\r' {
                    used.push((font.gid(ch), ch));
                }
            }
        }
    }
    used.sort_by_key(|(g, _)| *g);
    used.dedup_by_key(|(g, _)| *g);

    let (rasters, image_for_obj) =
        collect_unique_rasters(&document.objects, || edit_image::check_cancelled(cancel))?;

    let mut opacities: Vec<i32> = document
        .objects
        .iter()
        .map(|o| (o.opacity() * 100.0).round() as i32)
        .collect();
    opacities.push(100);
    opacities.sort_unstable();
    opacities.dedup();

    let mut b = PdfBuilder::new();

    let fontfile = b.begin();
    b.s(&format!(
        "{fontfile} 0 obj\n<< /Length {} /Length1 {} >>\nstream\n",
        font.data.len(),
        font.data.len()
    ));
    b.bytes(&font.data);
    b.s("\nendstream\nendobj\n");

    let sc = font.scale();
    let bb: [i32; 4] = font.bbox.map(|v| (v as f64 * sc).round() as i32);
    let ascent = (font.ascent as f64 * sc).round() as i32;
    let descent = (font.descent as f64 * sc).round() as i32;

    let desc = b.begin();
    b.s(&format!(
        "{desc} 0 obj\n<< /Type /FontDescriptor /FontName /NotoSans /Flags 32 \
         /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {ascent} /Descent {descent} \
         /CapHeight {ascent} /StemV 80 /FontFile2 {fontfile} 0 R >>\nendobj\n",
        bb[0], bb[1], bb[2], bb[3]
    ));

    let mut w_arr = String::new();
    for (gid, _) in &used {
        if *gid != 0 {
            w_arr.push_str(&format!("{gid} [{}] ", font.width(*gid) as i32));
        }
    }
    let cidfont = b.begin();
    b.s(&format!(
        "{cidfont} 0 obj\n<< /Type /Font /Subtype /CIDFontType2 /BaseFont /NotoSans \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
         /FontDescriptor {desc} 0 R /DW 600 /W [{w_arr}] /CIDToGIDMap /Identity >>\nendobj\n"
    ));

    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let pairs: Vec<(u16, char)> = used.iter().copied().filter(|(g, _)| *g != 0).collect();
    for chunk in pairs.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (gid, ch) in chunk {
            let cp = *ch as u32;
            if cp <= 0xFFFF {
                cmap.push_str(&format!("<{gid:04X}> <{cp:04X}>\n"));
            } else {
                let u = cp - 0x10000;
                let hi = 0xD800 + (u >> 10);
                let lo = 0xDC00 + (u & 0x3FF);
                cmap.push_str(&format!("<{gid:04X}> <{hi:04X}{lo:04X}>\n"));
            }
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let touni = b.begin();
    b.s(&format!(
        "{touni} 0 obj\n<< /Length {} >>\nstream\n{cmap}endstream\nendobj\n",
        cmap.len()
    ));

    let type0 = b.begin();
    b.s(&format!(
        "{type0} 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /NotoSans /Encoding /Identity-H \
         /DescendantFonts [{cidfont} 0 R] /ToUnicode {touni} 0 R >>\nendobj\n"
    ));

    let mut gs_ids: BTreeMap<i32, usize> = BTreeMap::new();
    for op100 in &opacities {
        let id = b.begin();
        let ca = (*op100 as f64) / 100.0;
        b.s(&format!(
            "{id} 0 obj\n<< /Type /ExtGState /ca {ca:.2} /CA {ca:.2} >>\nendobj\n"
        ));
        gs_ids.insert(*op100, id);
    }

    let mut img_ids: Vec<(usize, Option<usize>)> = Vec::new();
    for rast in &rasters {
        let smask_id = if rast.alpha.is_some() {
            let sid = b.begin();
            let a = rast.alpha.as_ref().unwrap();
            b.s(&format!(
                "{sid} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} >>\nstream\n",
                rast.w,
                rast.h,
                a.len()
            ));
            b.bytes(a);
            b.s("\nendstream\nendobj\n");
            Some(sid)
        } else {
            None
        };
        let iid = b.begin();
        let smask = smask_id
            .map(|s| format!(" /SMask {s} 0 R"))
            .unwrap_or_default();
        b.s(&format!(
            "{iid} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {}{smask} >>\nstream\n",
            rast.w,
            rast.h,
            rast.rgb.len()
        ));
        b.bytes(&rast.rgb);
        b.s("\nendstream\nendobj\n");
        img_ids.push((iid, smask_id));
    }

    let next_id = b.offs.len();
    let pages_id = next_id + n + n; // contents + page dicts, then Pages

    let mut content_ids = Vec::with_capacity(n);
    for (pi, geom) in geoms.iter().enumerate() {
        // Dest user space (origin 0): rotate=0 keeps (120,120) as 120,120 instead
        // of subtracting Trim origin. Rotation uses the visible (Crop∩Media)
        // unrotated size, not Trim — Trim⊂Crop + /Rotate 90/270 would otherwise
        // map stamps against the smaller box.
        let vis = [
            0.0,
            0.0,
            geom.visible[2] - geom.visible[0],
            geom.visible[3] - geom.visible[1],
        ];
        let rotate = geom.rotate;
        let mut content = String::new();
        for (oi, obj) in document.objects.iter().enumerate() {
            if obj.page_index() as usize != pi {
                continue;
            }
            if matches!(obj, EditObjectIn::Link { .. }) {
                continue;
            }
            let op100 = (obj.opacity() * 100.0).round() as i32;
            content.push_str(&format!("q\n/GS{op100} gs\n"));
            let rotated = push_object_rotate(
                &mut content,
                obj.object_rotate(),
                obj.overlay_aabb(vis, rotate),
            );
            match obj {
                EditObjectIn::Rect {
                    rect,
                    fill,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                    paint_path(
                        &mut content,
                        fill.as_deref(),
                        stroke.as_deref(),
                        *stroke_width,
                        &format!("{x:.2} {y:.2} {w:.2} {h:.2} re\n"),
                    );
                }
                EditObjectIn::Ellipse {
                    rect,
                    fill,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                    paint_path(
                        &mut content,
                        fill.as_deref(),
                        stroke.as_deref(),
                        *stroke_width,
                        &ellipse_path(x, y, w, h),
                    );
                }
                EditObjectIn::Triangle {
                    rect,
                    fill,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                    let pts = triangle_pts(x, y, w, h);
                    paint_path(
                        &mut content,
                        fill.as_deref(),
                        stroke.as_deref(),
                        *stroke_width,
                        &polygon_path(&pts),
                    );
                }
                EditObjectIn::Star {
                    rect,
                    fill,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                    let pts = star_pts(x, y, w, h);
                    paint_path(
                        &mut content,
                        fill.as_deref(),
                        stroke.as_deref(),
                        *stroke_width,
                        &polygon_path(&pts),
                    );
                }
                EditObjectIn::RoundRect {
                    rect,
                    fill,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                    paint_path(
                        &mut content,
                        fill.as_deref(),
                        stroke.as_deref(),
                        *stroke_width,
                        &round_rect_path(x, y, w, h),
                    );
                }
                EditObjectIn::Hexagon {
                    rect,
                    fill,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                    let pts = hexagon_pts(x, y, w, h);
                    paint_path(
                        &mut content,
                        fill.as_deref(),
                        stroke.as_deref(),
                        *stroke_width,
                        &polygon_path(&pts),
                    );
                }
                EditObjectIn::Bubble {
                    rect,
                    fill,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                    paint_path(
                        &mut content,
                        fill.as_deref(),
                        stroke.as_deref(),
                        *stroke_width,
                        &bubble_path(x, y, w, h),
                    );
                }
                EditObjectIn::Arrow {
                    rect,
                    fill,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                    let pts = arrow_pts(x, y, w, h);
                    paint_path(
                        &mut content,
                        fill.as_deref(),
                        stroke.as_deref(),
                        *stroke_width,
                        &polygon_path(&pts),
                    );
                }
                EditObjectIn::Text {
                    rect,
                    content: text,
                    font_size,
                    color,
                    align,
                    ..
                } => {
                    let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                    let fs = font_size.clamp(6.0, 96.0);
                    let (cr, cg, cb) = parse_hex(color.as_deref(), (0.12, 0.16, 0.22));
                    let lines = wrap_text(font, text, fs, w);
                    let mut ty = y + h - fs;
                    content.push_str(&format!("{cr:.3} {cg:.3} {cb:.3} rg\nBT\n/F1 {fs:.1} Tf\n"));
                    for line in lines {
                        let (hex, tw_em) = line_hex_and_width(font, &line);
                        let tw = tw_em * fs / 1000.0;
                        let tx = match align.as_deref() {
                            Some("center") => x + ((w - tw) / 2.0).max(0.0),
                            Some("right") => x + (w - tw).max(0.0),
                            _ => x,
                        };
                        if ty < y - fs {
                            break;
                        }
                        content.push_str(&format!("1 0 0 1 {tx:.2} {ty:.2} Tm\n<{hex}> Tj\n"));
                        ty -= fs * 1.25;
                    }
                    content.push_str("ET\n");
                }
                EditObjectIn::Image {
                    rect, keep_aspect, ..
                } => {
                    if let Some(ii) = image_for_obj[oi] {
                        let (x, y, w, h) = pdf_rect_to_overlay(rect, vis, rotate);
                        let rast = &rasters[ii];
                        let meet = keep_aspect.unwrap_or(true);
                        if meet {
                            let (dx, dy, dw, dh) =
                                image_meet_blit(x, y, w, h, rast.w as f64, rast.h as f64);
                            content.push_str(&format!(
                                "q\n{dw:.2} 0 0 {dh:.2} {dx:.2} {dy:.2} cm\n/Im{ii} Do\nQ\n"
                            ));
                        } else {
                            content.push_str(&format!(
                                "q\n{w:.2} 0 0 {h:.2} {x:.2} {y:.2} cm\n/Im{ii} Do\nQ\n"
                            ));
                        }
                    }
                }
                EditObjectIn::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    let (ax, ay) = pdf_point_to_overlay(*x1, *y1, vis, rotate);
                    let (bx, by) = pdf_point_to_overlay(*x2, *y2, vis, rotate);
                    let (sr, sg, sb) = parse_hex(stroke.as_deref(), (0.067, 0.094, 0.153));
                    let sw = stroke_width.unwrap_or(2.0).clamp(0.2, 24.0);
                    content.push_str(&format!(
                        "{sr:.3} {sg:.3} {sb:.3} RG\n{sw:.2} w 1 J\n{ax:.2} {ay:.2} m\n{bx:.2} {by:.2} l\nS\n"
                    ));
                }
                EditObjectIn::Ink {
                    points,
                    stroke,
                    stroke_width,
                    ..
                } => {
                    if points.len() >= 2 {
                        let (sr, sg, sb) = parse_hex(stroke.as_deref(), (0.067, 0.094, 0.153));
                        let sw = stroke_width.unwrap_or(2.5).clamp(0.2, 24.0);
                        content
                            .push_str(&format!("{sr:.3} {sg:.3} {sb:.3} RG\n{sw:.2} w 1 J 1 j\n"));
                        for (i, p) in points.iter().enumerate() {
                            let (x, y) = pdf_point_to_overlay(p.x, p.y, vis, rotate);
                            if i == 0 {
                                content.push_str(&format!("{x:.2} {y:.2} m\n"));
                            } else {
                                content.push_str(&format!("{x:.2} {y:.2} l\n"));
                            }
                        }
                        content.push_str("S\n");
                    }
                }
                EditObjectIn::Link { .. } => {}
            }
            if rotated {
                content.push_str("Q\n");
            }
            content.push_str("Q\n");
        }

        let cid = b.begin();
        b.s(&format!(
            "{cid} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n",
            content.len()
        ));
        content_ids.push(cid);
    }

    let mut out_page_ids = Vec::with_capacity(n);
    for (pi, geom) in geoms.iter().enumerate() {
        let rot = ((geom.rotate % 360) + 360) % 360;
        // Content uses absolute source coordinates transformed through a
        // zero-origin rotation window. Keep the correspondingly transformed
        // visible origin in the overlay boxes so qpdf maps both Forms 1:1.
        let page_box = overlay_page_box(geom.visible, rot);
        let (media, crop, trim) = (page_box, page_box, page_box);
        let mut xo = String::new();
        for (oi, obj) in document.objects.iter().enumerate() {
            if obj.page_index() as usize != pi {
                continue;
            }
            if let (EditObjectIn::Image { .. }, Some(ii)) = (obj, image_for_obj[oi]) {
                xo.push_str(&format!("/Im{ii} {} 0 R ", img_ids[ii].0));
            }
        }
        let mut gs_res = String::new();
        for op100 in gs_ids.keys() {
            gs_res.push_str(&format!("/GS{op100} {} 0 R ", gs_ids[op100]));
        }
        let uu = if (geom.user_unit - 1.0).abs() > 1e-6 {
            format!(" /UserUnit {:.4}", geom.user_unit)
        } else {
            String::new()
        };
        let pg = b.begin();
        b.s(&format!(
            "{pg} 0 obj\n<< /Type /Page /Parent {pages_id} 0 R \
             /MediaBox [{}] /CropBox [{}] /TrimBox [{}]{uu} \
             /Resources << /Font << /F1 {type0} 0 R >> /ExtGState << {gs_res}>> /XObject << {xo}>> >> \
             /Contents {} 0 R >>\nendobj\n",
            fmt_pdf_box(media),
            fmt_pdf_box(crop),
            fmt_pdf_box(trim),
            content_ids[pi]
        ));
        out_page_ids.push(pg);
    }

    let pages = b.begin();
    debug_assert_eq!(pages, pages_id);
    let kids = out_page_ids
        .iter()
        .map(|id| format!("{id} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    b.s(&format!(
        "{pages} 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {n} >>\nendobj\n"
    ));

    let catalog = b.begin();
    b.s(&format!(
        "{catalog} 0 obj\n<< /Type /Catalog /Pages {pages} 0 R >>\nendobj\n"
    ));

    let bytes = b.finish(catalog);
    std::fs::write(overlay_path, &bytes)
        .map_err(|e| AppError::output_not_writable(&format!("{overlay_path} ({e})")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PageGroup;
    use lopdf::Object;

    #[test]
    fn display_rotate_90_swaps_axes() {
        assert_eq!(
            unrotated_to_display(0.0, 0.0, 612.0, 792.0, 90),
            (0.0, 612.0)
        );
        assert_eq!(
            unrotated_to_display(612.0, 0.0, 612.0, 792.0, 90),
            (0.0, 0.0)
        );
    }

    #[test]
    fn overlay_rect_at_rotate_0_keeps_origin() {
        let vis = [72.0, 72.0, 540.0, 720.0];
        let r = PdfRectIn {
            x: 72.0,
            y: 72.0,
            w: 100.0,
            h: 50.0,
        };
        let (x, y, w, h) = pdf_rect_to_overlay(&r, vis, 0);
        assert!((x - 0.0).abs() < 1e-6);
        assert!((y - 0.0).abs() < 1e-6);
        assert!((w - 100.0).abs() < 1e-6);
        assert!((h - 50.0).abs() < 1e-6);
    }

    #[test]
    fn image_meet_letterboxes_wide_raster_in_tall_aabb() {
        let (dx, dy, dw, dh) = image_meet_blit(0.0, 0.0, 100.0, 200.0, 200.0, 100.0);
        assert!((dw - 100.0).abs() < 1e-6);
        assert!((dh - 50.0).abs() < 1e-6);
        assert!((dx - 0.0).abs() < 1e-6);
        assert!((dy - 75.0).abs() < 1e-6);
    }

    #[test]
    fn overlay_rect_against_full_trim_keeps_crop_offset() {
        // Visible crop origin (72,72) on a full-page TrimBox must stay at (72,72)
        // in overlay space so qpdf 1:1 maps onto dest TrimBox.
        let align = [0.0, 0.0, 612.0, 792.0];
        let r = PdfRectIn {
            x: 72.0,
            y: 72.0,
            w: 100.0,
            h: 50.0,
        };
        let (x, y, w, h) = pdf_rect_to_overlay(&r, align, 0);
        assert!((x - 72.0).abs() < 1e-6);
        assert!((y - 72.0).abs() < 1e-6);
        assert!((w - 100.0).abs() < 1e-6);
        assert!((h - 50.0).abs() < 1e-6);
    }

    #[test]
    fn hex_color_parses() {
        let (r, g, b) = parse_hex(Some("#ff0000"), (0.0, 0.0, 0.0));
        assert!((r - 1.0).abs() < 1e-6 && g < 1e-6 && b < 1e-6);
    }

    #[test]
    fn turkish_glyphs_exist_in_bundled_font() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/fonts/NotoSans-Regular.ttf");
        let data = std::fs::read(path).expect("bundled font");
        let font = FontInfo::parse(data).unwrap();
        for ch in "GİZLİ ŞğıİıĞğ".chars() {
            assert!(font.gid(ch) != 0, "missing glyph for {ch}");
        }
    }

    fn sample_link_obj() -> EditObjectIn {
        EditObjectIn::Link {
            page_index: 0,
            rect: PdfRectIn {
                x: 10.0,
                y: 20.0,
                w: 80.0,
                h: 40.0,
            },
            action: LinkActionIn::Uri {
                uri: "https://example.com/h8".into(),
            },
        }
    }

    fn sample_rect_obj() -> EditObjectIn {
        EditObjectIn::Rect {
            page_index: 0,
            rect: PdfRectIn {
                x: 10.0,
                y: 20.0,
                w: 80.0,
                h: 40.0,
            },
            fill: None,
            stroke: None,
            stroke_width: None,
            opacity: None,
            object_rotate: 0.0,
        }
    }

    #[test]
    fn validate_allows_empty() {
        let d = EditDocumentIn {
            version: 1,
            objects: vec![],
        };
        match validate_doc(&d) {
            Ok(()) => {}
            Err(err) => panic!("H6-doc: validate_doc([]) must be Ok, not {}", err.code),
        }
    }

    #[test]
    fn validate_allows_501_links() {
        let d = EditDocumentIn {
            version: 1,
            objects: (0..501).map(|_| sample_link_obj()).collect(),
        };
        match validate_doc(&d) {
            Ok(()) => {}
            Err(err) => panic!(
                "H8-links: 501 Link objects must pass validate_doc, not {}",
                err.code
            ),
        }
    }

    #[test]
    fn validate_rejects_501_paint() {
        let d = EditDocumentIn {
            version: 1,
            objects: (0..501).map(|_| sample_rect_obj()).collect(),
        };
        let err =
            validate_doc(&d).expect_err("H8-paint: 501 paint objects must be TOO_MANY_OBJECTS");
        assert_eq!(
            err.code, "TOO_MANY_OBJECTS",
            "H8-paint: 501 paint objects must be TOO_MANY_OBJECTS, not {}",
            err.code
        );
    }

    #[test]
    fn validate_allows_500_paint_plus_one_link() {
        let mut objects: Vec<EditObjectIn> = (0..500).map(|_| sample_rect_obj()).collect();
        objects.push(sample_link_obj());
        let d = EditDocumentIn {
            version: 1,
            objects,
        };
        match validate_doc(&d) {
            Ok(()) => {}
            Err(err) => panic!("H8-mix: 500 paint + 1 link must be Ok, not {}", err.code),
        }
    }

    #[test]
    fn validate_allows_5000_links() {
        let d = EditDocumentIn {
            version: 1,
            objects: (0..5000).map(|_| sample_link_obj()).collect(),
        };
        match validate_doc(&d) {
            Ok(()) => {}
            Err(err) => panic!("H8-mix: 5,000 links must be Ok, not {}", err.code),
        }
    }

    #[test]
    fn serde_roundtrip_text_kind() {
        let json = r##"{"version":1,"objects":[{"kind":"text","pageIndex":0,"rect":{"x":1,"y":2,"w":3,"h":4},"content":"GİZLİ","fontSize":14,"color":"#111827","align":"left","opacity":1}]}"##;
        let d: EditDocumentIn = serde_json::from_str(json).unwrap();
        assert_eq!(d.objects.len(), 1);
        match &d.objects[0] {
            EditObjectIn::Text { content, .. } => assert_eq!(content, "GİZLİ"),
            _ => panic!("expected text"),
        }
    }

    fn g(path: &str, pages: &str) -> PageGroup {
        PageGroup {
            path: path.to_string(),
            pages: pages.to_string(),
        }
    }

    #[test]
    fn overlay_args_identity_skips_empty_and_pages() {
        let args =
            build_edit_overlay_args(&[g("/a.pdf", "1-5")], &[5], "/ov.pdf", "/out.pdf").unwrap();
        assert_eq!(
            args,
            vec!["/a.pdf", "--overlay", "/ov.pdf", "--", "/out.pdf"]
        );
        assert!(!args.iter().any(|a| a == "--empty"));
        assert!(!args.iter().any(|a| a == "--pages"));
    }

    #[test]
    fn overlay_args_1_z_is_identity() {
        let args =
            build_edit_overlay_args(&[g("/a.pdf", "1-z")], &[12], "/ov.pdf", "/out.pdf").unwrap();
        assert!(!args.iter().any(|a| a == "--pages"));
        assert_eq!(args[0], "/a.pdf");
    }

    #[test]
    fn overlay_args_subset_uses_dot_pages() {
        let args =
            build_edit_overlay_args(&[g("/a.pdf", "2,4")], &[5], "/ov.pdf", "/out.pdf").unwrap();
        assert_eq!(
            args,
            vec![
                "/a.pdf",
                "--pages",
                ".",
                "2,4",
                "--",
                "--overlay",
                "/ov.pdf",
                "--",
                "/out.pdf"
            ]
        );
        assert!(!args.iter().any(|a| a == "--empty"));
    }

    #[test]
    fn overlay_args_multi_keeps_first_as_primary() {
        let args = build_edit_overlay_args(
            &[g("/a.pdf", "1-2"), g("/b.pdf", "1")],
            &[2, 1],
            "/ov.pdf",
            "/out.pdf",
        )
        .unwrap();
        assert_eq!(
            args,
            vec![
                "/a.pdf",
                "--pages",
                ".",
                "1-2",
                "/b.pdf",
                "1",
                "--",
                "--overlay",
                "/ov.pdf",
                "--",
                "/out.pdf"
            ]
        );
        assert!(!args.iter().any(|a| a == "--empty"));
    }

    #[test]
    fn expand_page_spec_keeps_order() {
        assert_eq!(expand_page_spec("1-z", 4).unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(expand_page_spec("3,1,2", 3).unwrap(), vec![3, 1, 2]);
        assert_eq!(expand_page_spec("z-1", 3).unwrap(), vec![3, 2, 1]);
    }

    fn sample_text_doc() -> EditDocumentIn {
        EditDocumentIn {
            version: 1,
            objects: vec![EditObjectIn::Text {
                page_index: 0,
                rect: PdfRectIn {
                    x: 72.0,
                    y: 700.0,
                    w: 200.0,
                    h: 24.0,
                },
                content: "Hello".into(),
                font_size: 14.0,
                color: Some("#111827".into()),
                align: None,
                opacity: None,
                object_rotate: 0.0,
            }],
        }
    }

    fn letter_geom() -> OverlayPageGeom {
        OverlayPageGeom {
            visible: [0.0, 0.0, 612.0, 792.0],
            media: [0.0, 0.0, 612.0, 792.0],
            crop: None,
            trim: None,
            rotate: 0,
            user_unit: 1.0,
            content_digest: content_digest(b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET"),
        }
    }

    #[test]
    fn overlay_page_box_keeps_transformed_offset_origin() {
        let visible = [72.0, 36.0, 540.0, 720.0];
        assert_eq!(overlay_page_box(visible, 0), [72.0, 36.0, 540.0, 720.0]);
        assert_eq!(overlay_page_box(visible, 90), [36.0, -72.0, 720.0, 396.0]);
        assert_eq!(overlay_page_box(visible, 180), [-72.0, -36.0, 396.0, 648.0]);
        assert_eq!(overlay_page_box(visible, 270), [-36.0, 72.0, 648.0, 540.0]);
    }

    fn test_font() -> FontInfo {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/fonts/NotoSans-Regular.ttf");
        FontInfo::parse(std::fs::read(path).expect("bundled font")).unwrap()
    }

    fn img_obj(path: &str, x: f64) -> EditObjectIn {
        EditObjectIn::Image {
            page_index: 0,
            rect: PdfRectIn {
                x,
                y: 400.0,
                w: 120.0,
                h: 80.0,
            },
            path: path.to_string(),
            opacity: None,
            object_rotate: 0.0,
            keep_aspect: Some(true),
        }
    }

    fn write_tiny_png(path: &Path, rgb: [u8; 3]) {
        image::RgbImage::from_pixel(8, 8, image::Rgb(rgb))
            .save(path)
            .unwrap();
    }

    fn tmp_root(prefix: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn overlay_dedupes_same_image_path_to_one_xobject() {
        let root = tmp_root("offpdf-edit-dedupe");
        let png = root.join("a.png");
        write_tiny_png(&png, [200, 10, 10]);
        let overlay = root.join("overlay.pdf");
        let path = png.to_str().unwrap();
        let doc = EditDocumentIn {
            version: 1,
            objects: vec![img_obj(path, 72.0), img_obj(path, 220.0)],
        };
        write_overlay_pdf(
            overlay.to_str().unwrap(),
            &[letter_geom()],
            &doc,
            &test_font(),
            None,
        )
        .unwrap();
        let bytes = std::fs::read(&overlay).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("/Subtype /Image").count(), 1);
        assert_eq!(text.matches("/Im0 Do").count(), 2);
        assert!(!text.contains("/Im1 Do"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overlay_embeds_distinct_images_separately() {
        let root = tmp_root("offpdf-edit-twoimg");
        let a = root.join("a.png");
        let b = root.join("b.png");
        write_tiny_png(&a, [200, 10, 10]);
        write_tiny_png(&b, [10, 10, 200]);
        let overlay = root.join("overlay.pdf");
        let doc = EditDocumentIn {
            version: 1,
            objects: vec![
                img_obj(a.to_str().unwrap(), 72.0),
                img_obj(b.to_str().unwrap(), 220.0),
            ],
        };
        write_overlay_pdf(
            overlay.to_str().unwrap(),
            &[letter_geom()],
            &doc,
            &test_font(),
            None,
        )
        .unwrap();
        let bytes = std::fs::read(&overlay).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("/Subtype /Image").count(), 2);
        assert_eq!(text.matches("/Im0 Do").count(), 1);
        assert_eq!(text.matches("/Im1 Do").count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overlay_rejects_oversized_jpeg_header() {
        let root = tmp_root("offpdf-edit-bigimg");
        let jpg = root.join("big.jpg");
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        b.extend_from_slice(&6000u16.to_be_bytes());
        b.extend_from_slice(&4000u16.to_be_bytes());
        b.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
        std::fs::write(&jpg, &b).unwrap();
        let overlay = root.join("overlay.pdf");
        let doc = EditDocumentIn {
            version: 1,
            objects: vec![img_obj(jpg.to_str().unwrap(), 72.0)],
        };
        let err = write_overlay_pdf(
            overlay.to_str().unwrap(),
            &[letter_geom()],
            &doc,
            &test_font(),
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, "IMAGE_TOO_LARGE");
        assert!(!overlay.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overlay_cancel_at_entry_returns_cancelled() {
        let root = tmp_root("offpdf-edit-cancel-entry");
        let png = root.join("a.png");
        write_tiny_png(&png, [10, 200, 10]);
        let overlay = root.join("overlay.pdf");
        let path = png.to_str().unwrap();
        let doc = EditDocumentIn {
            version: 1,
            objects: vec![img_obj(path, 72.0), img_obj("/does/not/matter.png", 220.0)],
        };
        let flag = AtomicBool::new(true);
        let err = write_overlay_pdf(
            overlay.to_str().unwrap(),
            &[letter_geom()],
            &doc,
            &test_font(),
            Some(&flag),
        )
        .unwrap_err();
        assert_eq!(err.code, "CANCELLED");
        assert!(!overlay.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overlay_cancel_after_first_embed_skips_next_image() {
        let root = tmp_root("offpdf-edit-cancel-mid");
        let a = root.join("a.png");
        write_tiny_png(&a, [10, 200, 10]);
        let missing = root.join("missing.png");
        let doc = EditDocumentIn {
            version: 1,
            objects: vec![
                img_obj(a.to_str().unwrap(), 72.0),
                img_obj(missing.to_str().unwrap(), 220.0),
            ],
        };
        let mut checks = 0u32;
        let err = collect_unique_rasters(&doc.objects, || {
            checks += 1;
            if checks >= 2 {
                Err(AppError::cancelled())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(err.code, "CANCELLED");
        assert_eq!(
            checks, 2,
            "must cancel on the post-embed check, not at entry"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn write_catalog_fixture(path: &Path) {
        use lopdf::{Dictionary, Object, Stream};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content_id = doc.add_object(Object::Stream(Stream::new(
            Dictionary::new(),
            b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET".to_vec(),
        )));
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", pages_id);
        page.set(
            "MediaBox",
            vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(612),
                Object::Integer(792),
            ],
        );
        page.set("Contents", content_id);
        let page_id = doc.add_object(Object::Dictionary(page));

        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Kids", vec![page_id.into()]);
        pages.set("Count", 1);
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        let mut item = Dictionary::new();
        item.set("Title", Object::string_literal("Chapter 1"));
        item.set("Dest", vec![page_id.into(), Object::Name(b"Fit".to_vec())]);
        let item_id = doc.add_object(Object::Dictionary(item));

        let mut outlines = Dictionary::new();
        outlines.set("Type", "Outlines");
        outlines.set("First", item_id);
        outlines.set("Last", item_id);
        outlines.set("Count", 1);
        let outlines_id = doc.add_object(Object::Dictionary(outlines));
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(item_id) {
            d.set("Parent", outlines_id);
        }

        let mut acro = Dictionary::new();
        acro.set("Fields", Vec::<Object>::new());
        let acro_id = doc.add_object(Object::Dictionary(acro));

        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", pages_id);
        catalog.set("Outlines", outlines_id);
        catalog.set("AcroForm", acro_id);
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", catalog_id);

        let mut info = Dictionary::new();
        info.set("Title", Object::string_literal("Fixture Doc"));
        info.set("Author", Object::string_literal("OffPDF"));
        let info_id = doc.add_object(Object::Dictionary(info));
        doc.trailer.set("Info", info_id);

        doc.save(path).expect("write catalog fixture");
    }

    fn test_qpdf() -> Option<std::path::PathBuf> {
        for c in [
            "/opt/homebrew/bin/qpdf",
            "/usr/local/bin/qpdf",
            "/opt/local/bin/qpdf",
            "/usr/bin/qpdf",
        ] {
            if Path::new(c).exists() {
                return Some(std::path::PathBuf::from(c));
            }
        }
        std::process::Command::new("qpdf")
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|_| std::path::PathBuf::from("qpdf"))
    }

    #[test]
    fn export_preserves_catalog_data_on_full_range_source() {
        let Some(qpdf) = test_qpdf() else {
            eprintln!("skip: qpdf not available");
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "offpdf-edit-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("src.pdf");
        let dest = root.join("out.pdf");
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        write_catalog_fixture(&src);
        let orig_bytes = std::fs::read(&src).unwrap();

        let font =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/fonts/NotoSans-Regular.ttf");
        let groups = [g(src.to_str().unwrap(), "1")];
        export_edit_pdf_with_runner(
            &groups,
            dest.to_str().unwrap(),
            &sample_text_doc(),
            &font,
            &work,
            "catalog",
            None,
            |args| {
                assert!(!args.iter().any(|a| a == "--empty"), "argv={args:?}");
                assert_eq!(args[0], src.to_str().unwrap());
                let out = std::process::Command::new(&qpdf)
                    .args(args)
                    .output()
                    .map_err(|e| AppError::io("qpdf failed to start", e))?;
                let code = out.status.code();
                if out.status.success() || code == Some(3) {
                    Ok(())
                } else {
                    Err(AppError::engine_failed(
                        String::from_utf8_lossy(&out.stderr).to_string(),
                    ))
                }
            },
        )
        .expect("export");

        assert_eq!(std::fs::read(&src).unwrap(), orig_bytes);

        let out = Document::load(&dest).expect("load dest");
        let info = match out.trailer.get(b"Info").ok() {
            Some(Object::Reference(id)) => out.get_dictionary(*id).ok().cloned(),
            Some(Object::Dictionary(d)) => Some(d.clone()),
            _ => None,
        }
        .expect("Info dict");
        let title = match info.get(b"Title").ok() {
            Some(Object::String(b, _)) => String::from_utf8_lossy(b).into_owned(),
            _ => String::new(),
        };
        assert!(title.contains("Fixture Doc"), "title={title:?}");

        let root_id = out.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let cat = out.get_dictionary(root_id).unwrap();
        assert!(cat.get(b"Outlines").is_ok(), "Outlines missing");
        assert!(cat.get(b"AcroForm").is_ok(), "AcroForm missing");
        assert!(std::fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .all(|e| !e.file_name().to_string_lossy().ends_with(".pdf.tmp")));

        let _ = std::fs::remove_dir_all(&root);
    }

    // L5 keepGreen: hard-linked dest stays OVERWRITE.
    #[test]
    fn export_rejects_hard_linked_destination() {
        let Some(qpdf) = test_qpdf() else {
            eprintln!("skip: qpdf not available");
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "offpdf-edit-hl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("src.pdf");
        let dest = root.join("alias.pdf");
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        write_catalog_fixture(&src);
        if std::fs::hard_link(&src, &dest).is_err() {
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        let before = std::fs::read(&src).unwrap();
        let font =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/fonts/NotoSans-Regular.ttf");
        let err = export_edit_pdf_with_runner(
            &[g(src.to_str().unwrap(), "1")],
            dest.to_str().unwrap(),
            &sample_text_doc(),
            &font,
            &work,
            "hl",
            None,
            |args| {
                let _ = qpdf;
                let _ = args;
                Ok(())
            },
        )
        .expect_err("hard-linked dest must be rejected");
        assert_eq!(err.code, "OVERWRITE");
        assert_eq!(std::fs::read(&src).unwrap(), before);
        let _ = std::fs::remove_dir_all(&root);
    }
}
