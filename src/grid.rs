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

    /// ピクセル矩形をグリッドセル座標に逆変換する（F0a キャプチャ用）。
    /// 戻り値: (col, row, col_span, row_span)
    pub fn rect_to_cell(&self, x: f64, y: f64, w: f64, h: f64) -> (u32, u32, u32, u32) {
        if self.base_cw <= 0 || self.base_ch <= 0 {
            return (0, 0, 1, 1);
        }
        let col = self.px_to_col_index(x.round() as i32) as u32;
        let row = self.py_to_row_index(y.round() as i32) as u32;
        // スパン: 右端・下端のセルインデックスから算出
        let col_end = self.px_to_col_index((x + w).round() as i32) as u32;
        let row_end = self.py_to_row_index((y + h).round() as i32) as u32;
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

    #[test]
    fn no_remainder_1980x1200() {
        let g = Grid::new(0, 0, 1980, 1200, 20, 12);
        assert_eq!(g.base_cw, 99);
        assert_eq!(g.base_ch, 100);
        assert_eq!(g.pad_left, 0);
        assert_eq!(g.pad_top, 0);
        // 全セル均等
        assert_eq!(g.cell_width_at(0), 99);
        assert_eq!(g.cell_width_at(19), 99);
        // 最右線は width-1 にクランプ
        let lines = g.grid_lines();
        assert_eq!(*lines.verticals.last().unwrap(), 1979);
        assert_eq!(*lines.horizontals.last().unwrap(), 1199);
    }

    #[test]
    fn remainder_absorb_both_edges() {
        // 1926 / 20 = 96 rem 6 → pad_left=3, pad_right=3
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        assert_eq!(g.base_cw, 96);
        assert_eq!(g.pad_left, 3);
        // 左端セル: 96+3=99, 右端セル: 96+3=99, 内部: 96
        assert_eq!(g.cell_width_at(0), 99);
        assert_eq!(g.cell_width_at(19), 99);
        assert_eq!(g.cell_width_at(5), 96);
        // グリッド線
        assert_eq!(g.col_to_x(0), 0);
        assert_eq!(g.col_to_x(1), 3 + 96);   // 99
        assert_eq!(g.col_to_x(2), 3 + 192);  // 195
        assert_eq!(g.col_to_x(20), 1926);
        // 最右線クランプ
        let lines = g.grid_lines();
        assert_eq!(*lines.verticals.last().unwrap(), 1925);
    }

    #[test]
    fn odd_remainder() {
        // 1921 / 20 = 96 rem 1 → pad_left=0, pad_right=1
        let g = Grid::new(0, 0, 1921, 1200, 20, 12);
        assert_eq!(g.pad_left, 0);
        assert_eq!(g.cell_width_at(0), 96);  // 左端: 96+0
        assert_eq!(g.cell_width_at(19), 97); // 右端: 96+1
    }

    #[test]
    fn snap_x_edge_cells() {
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        // 左端セル [0, 99) の中点付近
        assert_eq!(g.snap_x(50), 99);  // col 1 境界が近い
        assert_eq!(g.snap_x(48), 0);   // col 0 境界が近い
        // 最右端付近
        assert_eq!(g.snap_x(1920), 1926); // col 20 境界
    }

    #[test]
    fn cell_rect_edge() {
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        let r = g.cell_rect(0, 0, 1, 1);
        assert_eq!(r.x, 0);
        assert_eq!(r.w, 99); // 左端セル幅
        let r = g.cell_rect(19, 0, 1, 1);
        assert_eq!(r.w, 99); // 右端セル幅
        let r = g.cell_rect(5, 0, 1, 1);
        assert_eq!(r.w, 96); // 内部セル幅
    }

    #[test]
    fn cell_rect_span_across_edge() {
        let g = Grid::new(0, 0, 1926, 1200, 20, 12);
        // col 0-1 をまたぐ: 99 + 96 = 195
        let r = g.cell_rect(0, 0, 2, 1);
        assert_eq!(r.x, 0);
        assert_eq!(r.w, 195);
    }

    #[test]
    fn divides_evenly() {
        let g = Grid::new(0, 0, 2560, 1440, 20, 12);
        assert_eq!(g.base_cw, 128);
        assert_eq!(g.pad_left, 0);
        assert_eq!(g.cell_width_at(0), 128);
        assert_eq!(g.col_to_x(20), 2560);
    }
}