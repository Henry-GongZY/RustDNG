// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_tile_iterator.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_tile_iterator.h/.cpp — tile enumeration over an area.

use crate::dng_errors::DngResult;
use crate::dng_point::DngPoint;
use crate::dng_rect::DngRect;
use crate::dng_types::int32;
use crate::dng_utils::min_int32;

/// Port of `dng_tile_iterator` (`dng_tile_forward_iterator`).
#[derive(Debug, Clone)]
pub struct DngTileIterator {
    area: DngRect,

    tile_width: int32,
    tile_height: int32,

    tile_top: int32,
    tile_left: int32,

    row_left: int32,

    left_page: int32,
    right_page: int32,

    top_page: int32,
    bottom_page: int32,

    horizontal_page: int32,
    vertical_page: int32,
}

impl DngTileIterator {
    /// `dng_tile_iterator (image, area)` — needs the repeating tile and
    /// bounds without keeping a reference, so both are passed explicitly.
    pub fn new_image<I: crate::dng_image::DngImage + ?Sized>(image: &I, area: DngRect) -> Self {
        Self::new_tile(image.repeating_tile(), area.and_(image.bounds()))
    }

    /// `dng_tile_iterator (tileSize, area)`.
    pub fn new_tile_size(tile_size: DngPoint, area: DngRect) -> Self {
        let mut tile = area;
        tile.b = min_int32(tile.b, tile.t + tile_size.v);
        tile.r = min_int32(tile.r, tile.l + tile_size.h);

        Self::new_tile(tile, area)
    }

    /// `dng_tile_iterator (tile, area)`.
    pub fn new_tile(tile: DngRect, area: DngRect) -> Self {
        let mut iter = DngTileIterator {
            area: DngRect::new(),
            tile_width: 0,
            tile_height: 0,
            tile_top: 0,
            tile_left: 0,
            row_left: 0,
            left_page: 0,
            right_page: 0,
            top_page: 0,
            bottom_page: 0,
            horizontal_page: 0,
            vertical_page: 0,
        };
        iter.initialize(tile, area);
        iter
    }

    /// `Initialize (tile, area)`.
    fn initialize(&mut self, tile: DngRect, area: DngRect) {
        self.area = area;

        if area.is_empty() {
            self.vertical_page = 0;
            self.bottom_page = -1;
            return;
        }

        let v_offset = tile.t;
        let h_offset = tile.l;

        let tile_height = tile.b - v_offset;
        let tile_width = tile.r - h_offset;

        self.tile_height = tile_height;
        self.tile_width = tile_width;

        self.left_page = (self.area.l - h_offset) / tile_width;
        self.right_page = (self.area.r - h_offset - 1) / tile_width;

        self.horizontal_page = self.left_page;

        self.top_page = (self.area.t - v_offset) / tile_height;
        self.bottom_page = (self.area.b - v_offset - 1) / tile_height;

        self.vertical_page = self.top_page;

        self.tile_left = self.horizontal_page * tile_width + h_offset;
        self.tile_top = self.vertical_page * tile_height + v_offset;

        self.row_left = self.tile_left;
    }

    /// `GetOneTile (tile)`.
    pub fn get_one_tile(&mut self, tile: &mut DngRect) -> DngResult<bool> {
        if self.vertical_page > self.bottom_page {
            return Ok(false);
        }

        if self.vertical_page > self.top_page {
            tile.t = self.tile_top;
        } else {
            tile.t = self.area.t;
        }

        if self.vertical_page < self.bottom_page {
            tile.b = self.tile_top + self.tile_height;
        } else {
            tile.b = self.area.b;
        }

        if self.horizontal_page > self.left_page {
            tile.l = self.tile_left;
        } else {
            tile.l = self.area.l;
        }

        if self.horizontal_page < self.right_page {
            tile.r = self.tile_left + self.tile_width;
        } else {
            tile.r = self.area.r;
        }

        if self.horizontal_page < self.right_page {
            self.horizontal_page += 1;
            self.tile_left += self.tile_width;
        } else {
            self.vertical_page += 1;
            self.tile_top += self.tile_height;

            self.horizontal_page = self.left_page;
            self.tile_left = self.row_left;
        }

        Ok(true)
    }
}

/// Port of `dng_tile_reverse_iterator`.
pub struct DngTileReverseIterator {
    tiles: Vec<DngRect>,
    index: usize,
}

impl DngTileReverseIterator {
    pub fn new_image(image: &dyn crate::dng_image::DngImage, area: DngRect) -> Self {
        Self::new_tile(image.repeating_tile(), area)
    }

    pub fn new_tile_size(tile_size: DngPoint, area: DngRect) -> Self {
        let iterator = DngTileIterator::new_tile_size(tile_size, area);
        Self::from_forward(iterator)
    }

    pub fn new_tile(tile: DngRect, area: DngRect) -> Self {
        let iterator = DngTileIterator::new_tile(tile, area);
        Self::from_forward(iterator)
    }

    fn from_forward(mut iterator: DngTileIterator) -> Self {
        let mut tiles = Vec::new();
        let mut tile = DngRect::new();
        while iterator.get_one_tile(&mut tile).unwrap_or(false) {
            tiles.push(tile);
        }
        let index = tiles.len();
        DngTileReverseIterator { tiles, index }
    }

    /// `GetOneTile (tile)`.
    pub fn get_one_tile(&mut self, tile: &mut DngRect) -> bool {
        if self.index == 0 {
            return false;
        }

        self.index -= 1;
        *tile = self.tiles[self.index];

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_tile() {
        let area = DngRect::from_corners(0, 0, 10, 10);
        let mut iter = DngTileIterator::new_tile(area, area);

        let mut tile = DngRect::new();
        assert!(iter.get_one_tile(&mut tile).unwrap());
        assert_eq!(tile, area);
        assert!(!iter.get_one_tile(&mut tile).unwrap());
    }

    #[test]
    fn tiled_enumeration() {
        let area = DngRect::from_corners(0, 0, 10, 10);
        let tile = DngRect::from_corners(0, 0, 4, 4);

        let mut iter = DngTileIterator::new_tile(tile, area);

        let mut tiles = Vec::new();
        let mut t = DngRect::new();
        while iter.get_one_tile(&mut t).unwrap() {
            tiles.push(t);
        }

        assert_eq!(tiles.len(), 9);
        assert_eq!(tiles[0], DngRect::from_corners(0, 0, 4, 4));
        assert_eq!(tiles[1], DngRect::from_corners(0, 4, 4, 8));
        assert_eq!(tiles[2], DngRect::from_corners(0, 8, 4, 10));
        assert_eq!(tiles[3], DngRect::from_corners(4, 0, 8, 4));
        assert_eq!(tiles[8], DngRect::from_corners(8, 8, 10, 10));
    }

    #[test]
    fn offset_area_tiles() {
        // Area not aligned to tile origin.
        let area = DngRect::from_corners(5, 5, 15, 15);
        let tile = DngRect::from_corners(4, 4, 8, 8);

        let mut iter = DngTileIterator::new_tile(tile, area);

        let mut tiles = Vec::new();
        let mut t = DngRect::new();
        while iter.get_one_tile(&mut t).unwrap() {
            tiles.push(t);
            assert!(area.contains_rect(t));
        }

        assert_eq!(tiles.len(), 9);
        assert_eq!(tiles[0], DngRect::from_corners(5, 5, 8, 8));
        assert_eq!(tiles[1], DngRect::from_corners(5, 8, 8, 12));
        assert_eq!(tiles[2], DngRect::from_corners(5, 12, 8, 15));
        assert_eq!(tiles[8], DngRect::from_corners(12, 12, 15, 15));
    }

    #[test]
    fn reverse_iterator() {
        let area = DngRect::from_corners(0, 0, 8, 8);
        let tile = DngRect::from_corners(0, 0, 4, 4);

        let mut iter = DngTileReverseIterator::new_tile(tile, area);

        let mut count = 0;
        let mut t = DngRect::new();
        while iter.get_one_tile(&mut t) {
            count += 1;
        }
        assert_eq!(count, 4);
    }

    #[test]
    fn tile_size_constructor() {
        let area = DngRect::from_corners(0, 0, 9, 9);
        let mut iter = DngTileIterator::new_tile_size(DngPoint::new(3, 5), area);

        let mut tiles = Vec::new();
        let mut t = DngRect::new();
        while iter.get_one_tile(&mut t).unwrap() {
            tiles.push(t);
        }

        assert_eq!(tiles.len(), 6); // 2 rows of 3 cols
        assert_eq!(tiles[0], DngRect::from_corners(0, 0, 3, 5));
        assert_eq!(tiles[1], DngRect::from_corners(0, 5, 3, 9));
        assert_eq!(tiles[5], DngRect::from_corners(6, 5, 9, 9));
    }
}
