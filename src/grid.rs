/// グリッド座標計算。整数演算のみ（NF1要件）。
/// 端数はモニター両端のセルに吸収し、内部セルは均等幅を保つ。

/// 1モニター分のグリッド情報
#[derive(Debug, Clone)]
pub struct Grid {
    /// モニター左上の物理ピクセル座標
    pub origin_x: i32,
    pub origin_y: i32,
    /// モニターの幅・高さ（物理ピクセル）
    pub width: i32,
    pub height: i32,
    /// グリッド列数・行数
    pub columns: i32,
    pub rows: i32,
    /// 端数吸収パディング（左・上）
    pad_left: i32,
    pad_top: i32,
    /// 内部セルの基本幅・高さ（均等）
    base_cw: i32,
    base_ch: i32,
}

impl Grid {
    pub fn new(
        origin_x: i32,
        origin_y: i32,
        width: i32,
        height: i32,
        columns: u32,
        rows: u32,
    ) -> Self {
        let cols = columns as i32;
        let rows_i = rows as i32;

        let base_cw = width / cols;
        let rem_x = width % cols;
        let pad_left = rem_x / 2;

        let base_ch = height / rows_i;
        let rem_y = height % rows_i;
        let pad_top = rem_y / 2;

        Self {
            origin_x,
            origin_y,
            width,
            height,
            columns: cols,
            rows: rows_i,
            pad_left,
            pad_top,
            base_cw,
            base_ch,
        }
    }

    // ── セル幅・高さ ──

    /// 内部セルの基本幅（端セルはこれ + パディング分）
    pub fn cell_width(&self) -> i32 {
        self.base_cw
    }

    /// 内部セルの基本高さ
    pub fn cell_height(&self) -> i32 {
        self.base_ch
    }

    /// 指定列の実幅（端セルは端数分だけ広い）
    pub fn cell_width_at(&self, col: i32) -> i32 {
        if col == 0 {
            self.base_cw + self.pad_left
        } else if col == self.columns - 1 {
            self.base_cw + (self.width % self.columns) - self.pad_left
        } else {
            self.base_cw
        }
    }

    /// 指定行の実高さ
    pub fn cell_height_at(&self, row: i32) -> i32 {
        if row == 0 {
            self.base_ch + self.pad_top
        } else if row == self.rows - 1 {
            self.base_ch + (self.height % self.rows) - self.pad_top
        } else {
            self.base_ch
        }
    }

    // ── 座標変換 ──

    /// グリッド列インデックス(0-based) → 左端X座標（絶対）
    /// col = columns のとき右端境界（= origin_x + width）を返す。
    pub fn col_to_x(&self, col: i32) -> i32 {
        let col = col.clamp(0, self.columns);
        if col == 0 {
            self.origin_x
        } else if col == self.columns {
            self.origin_x + self.width
        } else {
            self.origin_x + self.pad_left + col * self.base_cw
        }
    }

    /// グリッド行インデックス(0-based) → 上端Y座標（絶対）
    pub fn row_to_y(&self, row: i32) -> i32 {
        let row = row.clamp(0, self.rows);
        if row == 0 {
            self.origin_y
        } else if row == self.rows {
            self.origin_y + self.height
        } else {
            self.origin_y + self.pad_top + row * self.base_ch
        }
    }

    /// 物理ピクセルX座標を最寄りのグリッド列境界にスナップする。
    pub fn snap_x(&self, px: i32) -> i32 {
        let col = self.px_to_col_index(px);
        let left = self.col_to_x(col);
        let right = self.col_to_x(col + 1);
        if (px - left).abs() <= (px - right).abs() { left } else { right }
    }

    /// 物理ピクセルY座標を最寄りのグリッド行境界にスナップする。
    pub fn snap_y(&self, py: i32) -> i32 {
        let row = self.py_to_row_index(py);
        let top = self.row_to_y(row);
        let bottom = self.row_to_y(row + 1);
        if (py - top).abs() <= (py - bottom).abs() { top } else { bottom }
    }

    /// 物理ピクセルX → 所属セルの列インデックス (0..columns-1)
    fn px_to_col_index(&self, px: i32) -> i32 {
        let rel = px - self.origin_x;
        if rel < 0 { return 0; }
        if rel >= self.width { return self.columns - 1; }
        // 左端セル: [0, pad_left + base_cw)
        let left_edge = self.pad_left + self.base_cw;
        if rel < left_edge {
            return 0;
        }
        // 内部セル: col = (rel - pad_left) / base_cw, clamped
        let est = (rel - self.pad_left) / self.base_cw;
        est.clamp(1, self.columns - 1)
    }

    /// 物理ピクセルY → 所属セルの行インデックス (0..rows-1)
    fn py_to_row_index(&self, py: i32) -> i32 {
        let rel = py - self.origin_y;
        if rel < 0 { return 0; }
        if rel >= self.height { return self.rows - 1; }
        let top_edge = self.pad_top + self.base_ch;
        if rel < top_edge {
            return 0;
        }
        let est = (rel - self.pad_top) / self.base_ch;
        est.clamp(1, self.rows - 1)
    }

    // ── 矩形 ──

    /// グリッドセルの絶対座標とサイズを返す（col_span列分、row_span行分）。
    pub fn cell_rect(&self, col: u32, row: u32, col_span: u32, row_span: u32) -> CellRect {
        let x = self.col_to_x(col as i32);
        let x_end = self.col_to_x((col + col_span) as i32);
        let y = self.row_to_y(row as i32);
        let y_end = self.row_to_y((row + row_span) as i32);
        CellRect { x, y, w: x_end - x, h: y_end - y }
    }

    /// ピクセル矩形をグリッドセル座標に逆変換する。
    /// 各辺を最寄りグリッド境界に丸める（四捨五入）ため、
    /// 端数はみ出しが半セル未満なら縮小方向に倒れる。
    /// 戻り値: (col, row, col_span, row_span)
    pub fn rect_to_cell(&self, x: f64, y: f64, w: f64, h: f64) -> (u32, u32, u32, u32) {
        if self.base_cw <= 0 || self.base_ch <= 0 {
            return (0, 0, 1, 1);
        }

        // 4辺を最寄りグリッド線にスナップ
        let snapped_left = self.snap_x(x.round() as i32);
        let snapped_top = self.snap_y(y.round() as i32);
        let snapped_right = self.snap_x((x + w).round() as i32);
        let snapped_bottom = self.snap_y((y + h).round() as i32);

        // スナップ後の左上 → セルインデックス
        let col = self.px_to_col_index(snapped_left) as u32;
        let row = self.py_to_row_index(snapped_top) as u32;

        // スナップ後の右下 → セルインデックス（-1 で境界上を手前セルに倒す）
        let col_end = self.px_to_col_index((snapped_right - 1).max(snapped_left)) as u32;
        let row_end = self.py_to_row_index((snapped_bottom - 1).max(snapped_top)) as u32;

        let col_span = (col_end - col + 1).max(1);
        let row_span = (row_end - row + 1).max(1);
        (col, row, col_span, row_span)
    }

    // ── オーバーレイ用 ──

    /// すべてのグリッド交点の絶対座標リストを返す（オーバーレイ描画用）。
    /// 最右線・最下線は width-1 / height-1 にクランプし、画面内に収める。
    pub fn grid_lines(&self) -> GridLines {
        let verticals: Vec<i32> = (0..=self.columns)
            .map(|c| {
                let x = self.col_to_x(c);
                x.min(self.origin_x + self.width - 1)
            })
            .collect();
        let horizontals: Vec<i32> = (0..=self.rows)
            .map(|r| {
                let y = self.row_to_y(r);
                y.min(self.origin_y + self.height - 1)
            })
            .collect();
        GridLines {
            verticals,
            horizontals,
            origin_x: self.origin_x,
            origin_y: self.origin_y,
            width: self.width,
            height: self.height,
        }
    }
}

/// スナップ後の矩形
#[derive(Debug, Clone, Copy)]
pub struct CellRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// オーバーレイ描画用グリッド線情報
#[derive(Debug, Clone)]
pub struct GridLines {
    /// 垂直線のX座標リスト（絶対）
    pub verticals: Vec<i32>,
    /// 水平線のY座標リスト（絶対）
    pub horizontals: Vec<i32>,
    /// モニター領域
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ================================================================
    // 1. Grid::new — 基本構築
    // ================================================================

    #[test]
    fn new_evenly_divisible() {
        // 2560 / 20 = 128, 1440 / 12 = 120 → 端数なし
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        assert_eq!(g.cell_width(), 128);
        assert_eq!(g.cell_height(), 120);
        assert_eq!(g.col_to_x(0), 0);
        assert_eq!(g.col_to_x(20), 2560);
        assert_eq!(g.row_to_y(12), 1440);
    }

    #[test]
    fn new_with_even_remainder() {
        // 1926 / 20 = 96 rem 6 → pad_left = 3
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        assert_eq!(g.cell_width(), 96);
        assert_eq!(g.cell_width_at(0), 99);  // 96 + 3
        assert_eq!(g.cell_width_at(19), 99); // 96 + (6-3)
        assert_eq!(g.cell_width_at(10), 96); // 内部セル
    }

    #[test]
    fn new_with_odd_remainder() {
        // 1921 / 20 = 96 rem 1 → pad_left = 0, 右端 +1
        let g = Grid::new(0, 0, 1921, 1200, 20, 12);
        assert_eq!(g.cell_width_at(0), 96);
        assert_eq!(g.cell_width_at(19), 97);
    }

    #[test]
    fn new_with_nonzero_origin() {
        // マルチモニター: セカンダリが x=2560 から始まる
        let g = Grid::new(2560, 0, 1920, 1080, 10, 6);
        assert_eq!(g.col_to_x(0), 2560);
        assert_eq!(g.col_to_x(10), 2560 + 1920);
        assert_eq!(g.row_to_y(0), 0);
        assert_eq!(g.row_to_y(6), 1080);
    }

    #[test]
    fn new_with_negative_origin() {
        // 左側モニター: x = -1920
        let g = Grid::new(-1920, 0, 1920, 1080, 8, 4);
        assert_eq!(g.col_to_x(0), -1920);
        assert_eq!(g.col_to_x(8), 0);
    }

    #[test]
    fn new_minimum_grid_1x1() {
        let g = Grid::new(0, 0, 1920, 1080, 1, 1);
        assert_eq!(g.cell_width(), 1920);
        assert_eq!(g.cell_height(), 1080);
        assert_eq!(g.col_to_x(0), 0);
        assert_eq!(g.col_to_x(1), 1920);
    }

    #[test]
    fn new_large_grid_100x100() {
        let g = Grid::new(0, 0, 3840, 2160, 100, 100);
        assert_eq!(g.cell_width(), 38);  // 3840 / 100 = 38 rem 40
        assert_eq!(g.col_to_x(100), 3840);
        assert_eq!(g.row_to_y(100), 2160);
    }

    // ================================================================
    // 2. col_to_x / row_to_y — 座標変換
    // ================================================================

    #[test]
    fn col_to_x_clamps_negative() {
        let g = Grid::new(0, 0, 1920, 1080, 10, 6);
        assert_eq!(g.col_to_x(-5), g.col_to_x(0));
    }

    #[test]
    fn col_to_x_clamps_over_max() {
        let g = Grid::new(0, 0, 1920, 1080, 10, 6);
        assert_eq!(g.col_to_x(15), g.col_to_x(10));
    }

    #[test]
    fn col_to_x_boundary_consistency() {
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        let mut prev = g.col_to_x(0);
        for c in 1..=20 {
            let cur = g.col_to_x(c);
            assert!(cur > prev, "col_to_x({}) = {} <= col_to_x({}) = {}", c, cur, c - 1, prev);
            prev = cur;
        }
    }

    #[test]
    fn row_to_y_boundary_consistency() {
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        let mut prev = g.row_to_y(0);
        for r in 1..=12 {
            let cur = g.row_to_y(r);
            assert!(cur > prev);
            prev = cur;
        }
    }

    // ================================================================
    // 3. snap_x / snap_y — ピクセル→最寄りグリッド線
    // ================================================================

    #[test]
    fn snap_x_exact_on_grid_line() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        assert_eq!(g.snap_x(640), 640);
    }

    #[test]
    fn snap_x_midpoint_rounds_left() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        // col 5 = 640, col 6 = 768, midpoint = 704
        assert_eq!(g.snap_x(704), 640);
    }

    #[test]
    fn snap_x_just_past_midpoint_rounds_right() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        assert_eq!(g.snap_x(705), 768);
    }

    #[test]
    fn snap_x_at_origin() {
        let g = Grid::new(100, 0, 1920, 1080, 10, 6);
        assert_eq!(g.snap_x(100), 100);
    }

    #[test]
    fn snap_x_before_origin() {
        let g = Grid::new(100, 0, 1920, 1080, 10, 6);
        assert_eq!(g.snap_x(50), 100);
    }

    #[test]
    fn snap_x_beyond_right_edge() {
        let g = Grid::new(0, 0, 1920, 1080, 10, 6);
        assert_eq!(g.snap_x(2000), 1920);
    }

    #[test]
    fn snap_y_edge_cells_with_padding() {
        let g = Grid::new(0, 0, 1926, 1201, 20, 12);
        let row1_y = g.row_to_y(1);
        let mid = row1_y / 2;
        let snapped = g.snap_y(mid);
        assert!(snapped == 0 || snapped == row1_y);
    }

    // ================================================================
    // 4. cell_rect — セル矩形
    // ================================================================

    #[test]
    fn cell_rect_single_cell_at_origin() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let r = g.cell_rect(0, 0, 1, 1);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.w, 128);
        assert_eq!(r.h, 120);
    }

    #[test]
    fn cell_rect_full_screen() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let r = g.cell_rect(0, 0, 20, 12);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.w, 2560);
        assert_eq!(r.h, 1440);
    }

    #[test]
    fn cell_rect_spans_edge_and_internal() {
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        let r = g.cell_rect(0, 0, 3, 1);
        assert_eq!(r.w, g.col_to_x(3) - g.col_to_x(0));
    }

    #[test]
    fn cell_rect_with_offset_origin() {
        let g = Grid::new(2560, 100, 1920, 1080, 10, 6);
        let r = g.cell_rect(0, 0, 1, 1);
        assert_eq!(r.x, 2560);
        assert_eq!(r.y, 100);
    }

    #[test]
    fn cell_rect_last_cell() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let r = g.cell_rect(19, 11, 1, 1);
        assert_eq!(r.x + r.w, 2560);
        assert_eq!(r.y + r.h, 1440);
    }

    #[test]
    fn cell_rect_sum_equals_total() {
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        let total_w: i32 = (0..20).map(|c| g.cell_width_at(c)).sum();
        assert_eq!(total_w, 1926);
        let total_h: i32 = (0..12).map(|r| g.cell_height_at(r)).sum();
        assert_eq!(total_h, 1200);
    }

    // ================================================================
    // 5. rect_to_cell — ピクセル矩形→セル座標（F0a キャプチャ）
    // ================================================================

    #[test]
    fn rect_to_cell_exact_alignment() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let r = g.cell_rect(2, 1, 3, 2);
        let (col, row, cs, rs) = g.rect_to_cell(r.x as f64, r.y as f64, r.w as f64, r.h as f64);
        assert_eq!((col, row, cs, rs), (2, 1, 3, 2));
    }

    #[test]
    fn rect_to_cell_slight_offset() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let r = g.cell_rect(5, 3, 2, 2);
        let (col, row, cs, rs) = g.rect_to_cell(
            (r.x + 3) as f64,
            (r.y + 3) as f64,
            (r.w - 6) as f64,
            (r.h - 6) as f64,
        );
        assert_eq!((col, row), (5, 3));
    }

    #[test]
    fn rect_to_cell_full_screen() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let (col, row, cs, rs) = g.rect_to_cell(0.0, 0.0, 2560.0, 1440.0);
        assert_eq!((col, row, cs, rs), (0, 0, 20, 12));
    }

    #[test]
    fn rect_to_cell_zero_size_returns_minimum() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let (_, _, cs, rs) = g.rect_to_cell(500.0, 500.0, 0.0, 0.0);
        assert!(cs >= 1);
        assert!(rs >= 1);
    }

    #[test]
    fn rect_to_cell_with_nonzero_origin() {
        let g = Grid::new(2560, 0, 1920, 1080, 10, 6);
        let r = g.cell_rect(0, 0, 5, 3);
        let (col, row, cs, rs) = g.rect_to_cell(r.x as f64, r.y as f64, r.w as f64, r.h as f64);
        assert_eq!((col, row, cs, rs), (0, 0, 5, 3));
    }

    #[test]
    fn rect_to_cell_shrinks_small_overshoot() {
        // 右端が次セルに数px はみ出し → 縮小方向に丸まる（スパン増えない）
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let r = g.cell_rect(5, 3, 3, 2);
        // 右端・下端を 10px はみ出させる（セル幅128, 高さ120 の半分未満）
        let (col, row, cs, rs) = g.rect_to_cell(
            r.x as f64, r.y as f64,
            (r.w + 10) as f64, (r.h + 10) as f64,
        );
        assert_eq!((col, row, cs, rs), (5, 3, 3, 2), "small overshoot should not increase span");
    }

    #[test]
    fn rect_to_cell_expands_large_overshoot() {
        // 右端がセル幅の半分以上はみ出し → 拡大方向に丸まる
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let r = g.cell_rect(5, 3, 3, 2);
        // セル幅128 の半分超 = 65px 以上はみ出し
        let (col, row, cs, rs) = g.rect_to_cell(
            r.x as f64, r.y as f64,
            (r.w + 70) as f64, (r.h + 65) as f64,
        );
        assert_eq!((cs, rs), (4, 3), "large overshoot should increase span");
    }

    #[test]
    fn rect_to_cell_stable_after_roundtrip() {
        // cell_rect → rect_to_cell → cell_rect が冪等であることを全セルで検証
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        for col in 0..18 {
            for row in 0..10 {
                let r1 = g.cell_rect(col, row, 3, 2);
                let (c, r, cs, rs) = g.rect_to_cell(r1.x as f64, r1.y as f64, r1.w as f64, r1.h as f64);
                let r2 = g.cell_rect(c, r, cs, rs);
                assert_eq!((r1.x, r1.y, r1.w, r1.h), (r2.x, r2.y, r2.w, r2.h),
                    "roundtrip unstable at col={}, row={}", col, row);
            }
        }
    }

    #[test]
    fn rect_to_cell_dwm_drift_tolerance() {
        // DWM ボーダー補償で 1-2px ずれても正規化済みサイズが維持される
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        let r = g.cell_rect(4, 2, 5, 3);
        for dx in -2i32..=2 {
            for dy in -2i32..=2 {
                let (_, _, cs, rs) = g.rect_to_cell(
                    (r.x + dx) as f64, (r.y + dy) as f64,
                    (r.w - dx) as f64, (r.h - dy) as f64,
                );
                assert_eq!((cs, rs), (5, 3),
                    "DWM drift ({},{}) should not change span", dx, dy);
            }
        }
    }

    // ================================================================
    // 6. grid_lines — オーバーレイ描画用
    // ================================================================

    #[test]
    fn grid_lines_count() {
        let g = Grid::new(0, 0, 1920, 1080, 10, 6);
        let lines = g.grid_lines();
        assert_eq!(lines.verticals.len(), 11);
        assert_eq!(lines.horizontals.len(), 7);
    }

    #[test]
    fn grid_lines_first_and_last() {
        let g = Grid::new(0, 0, 1920, 1080, 10, 6);
        let lines = g.grid_lines();
        assert_eq!(lines.verticals[0], 0);
        assert_eq!(*lines.verticals.last().unwrap(), 1919);
        assert_eq!(lines.horizontals[0], 0);
        assert_eq!(*lines.horizontals.last().unwrap(), 1079);
    }

    #[test]
    fn grid_lines_monotonically_increasing() {
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        let lines = g.grid_lines();
        for w in lines.verticals.windows(2) {
            assert!(w[1] >= w[0]);
        }
        for w in lines.horizontals.windows(2) {
            assert!(w[1] >= w[0]);
        }
    }

    #[test]
    fn grid_lines_within_monitor_bounds() {
        let g = Grid::new(100, 200, 1920, 1080, 10, 6);
        let lines = g.grid_lines();
        for &x in &lines.verticals {
            assert!(x >= 100 && x < 100 + 1920);
        }
        for &y in &lines.horizontals {
            assert!(y >= 200 && y < 200 + 1080);
        }
    }

    // ================================================================
    // 7. snap ラウンドトリップ整合性
    // ================================================================

    #[test]
    fn snap_roundtrip_all_grid_lines() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        for c in 0..=20 {
            let x = g.col_to_x(c);
            assert_eq!(g.snap_x(x), x, "snap_x roundtrip failed for col {}", c);
        }
        for r in 0..=12 {
            let y = g.row_to_y(r);
            assert_eq!(g.snap_y(y), y, "snap_y roundtrip failed for row {}", r);
        }
    }

    #[test]
    fn snap_always_lands_on_grid_line() {
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        let valid_xs: Vec<i32> = (0..=20).map(|c| g.col_to_x(c)).collect();
        for px in (0..1926).step_by(7) {
            let snapped = g.snap_x(px);
            assert!(
                valid_xs.contains(&snapped),
                "snap_x({}) = {} is not a valid grid line",
                px,
                snapped
            );
        }
    }
}