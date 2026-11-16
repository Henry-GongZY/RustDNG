// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_area_task.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_area_task.h/.cpp + dng_filter_task.h/.cpp — area-parallel task
// execution.
//
// Binding deviation (coordinator + ARCHITECTURE.md §9): the Rust port
// executes tasks SERIALLY (single-threaded loop) — `Perform` runs the exact
// C++ tile-decomposition order on thread index 0.  All C++ names
// (Start/Process/Finish/FindTileSize/ProcessOnThread) are preserved, and a
// `process_on_single_thread` convenience mirrors `dng_area_task::Perform`
// minus threading.  `dng_range_parallel_task::Do` maps to a serial loop.
// See DESIGN.md.

use crate::dng_errors::DngResult;
use crate::dng_image::{DngImage, EdgeOption};
use crate::dng_pixel_buffer::DngPixelBuffer;
use crate::dng_point::DngPoint;
use crate::dng_rect::DngRect;
use crate::dng_sdk_limits::kMaxMPThreads;
use crate::dng_string::DngString;
use crate::dng_types::{int32, real64, uint32};
use crate::dng_utils::{max_int32, max_uint32_3, min_int32, min_uint32_3, round_int32_f64};
use std::sync::Arc;

/// Port of `dng_area_task_progress`.
pub trait DngAreaTaskProgress: Send + Sync {
    /// `FinishedTile (tile)`.
    fn finished_tile(&self, tile: &DngRect);
}

/// Port of `dng_area_task` (abstract base in C++).
pub trait DngAreaTask: Send + Sync {
    /// `Name ()`.
    fn name(&self) -> &str {
        "unnamed dng_area_task"
    }

    /// `MaxThreads ()`.
    fn max_threads(&self) -> uint32 {
        kMaxMPThreads
    }

    /// `MinTaskArea ()`.
    fn min_task_area(&self) -> uint32 {
        4096
    }

    /// `UnitCell ()`.
    fn unit_cell(&self) -> DngPoint {
        DngPoint::new(1, 1)
    }

    /// `MaxTileSize ()`.
    fn max_tile_size(&self) -> DngPoint {
        DngPoint::new(1024, 1024)
    }

    /// `RepeatingTile1 ()`.
    fn repeating_tile1(&self) -> DngRect {
        DngRect::new()
    }

    /// `RepeatingTile2 ()`.
    fn repeating_tile2(&self) -> DngRect {
        DngRect::new()
    }

    /// `RepeatingTile3 ()`.
    fn repeating_tile3(&self) -> DngRect {
        DngRect::new()
    }

    /// `Start (threadCount, dstArea, tileSize, allocator, sniffer)`.
    #[allow(unused_variables)]
    fn start(
        &mut self,
        thread_count: uint32,
        dst_area: DngRect,
        tile_size: DngPoint,
        sniffer: Option<&Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        Ok(())
    }

    /// `Process (threadIndex, tile, sniffer)` — the per-tile work.
    fn process(
        &mut self,
        thread_index: uint32,
        tile: DngRect,
        sniffer: Option<&Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()>;

    /// `Finish (threadCount)`.
    fn finish(&mut self, _thread_count: uint32) {}

    // -------------------------------------------------- serial dispatch

    /// `FindTileSize (area)` — exact C++ port.
    fn find_tile_size(&self, area: DngRect) -> DngPoint {
        let mut repeating_tile1 = self.repeating_tile1();
        let mut repeating_tile2 = self.repeating_tile2();
        let mut repeating_tile3 = self.repeating_tile3();

        if repeating_tile1.is_empty() {
            repeating_tile1 = area;
        }
        if repeating_tile2.is_empty() {
            repeating_tile2 = area;
        }
        if repeating_tile3.is_empty() {
            repeating_tile3 = area;
        }

        let repeat_v = min_uint32_3(repeating_tile1.h(), repeating_tile2.h(), repeating_tile3.h());
        let repeat_h = min_uint32_3(repeating_tile1.w(), repeating_tile2.w(), repeating_tile3.w());

        let max_tile_size = self.max_tile_size();

        let mut tile_size = DngPoint::default();

        tile_size.v = min_int32(repeat_v as int32, max_tile_size.v);
        tile_size.h = min_int32(repeat_h as int32, max_tile_size.h);

        tile_size.v = max_int32(tile_size.v, 1);
        tile_size.h = max_int32(tile_size.h, 1);

        // Adjust tile size so tiles cover the repeat area evenly (see the
        // C++ comment about the image core team).
        let mut count_v = (repeat_v + tile_size.v as uint32 - 1) / tile_size.v as uint32;
        let mut count_h = (repeat_h + tile_size.h as uint32 - 1) / tile_size.h as uint32;

        count_v = count_v.max(1);
        count_h = count_h.max(1);

        tile_size.v = ((repeat_v + count_v - 1) / count_v) as int32;
        tile_size.h = ((repeat_h + count_h - 1) / count_h) as int32;

        // Round up to unit cell size.
        let unit_cell = self.unit_cell();

        if unit_cell.h != 1 || unit_cell.v != 1 {
            tile_size.v = ((tile_size.v + unit_cell.v - 1) / unit_cell.v) * unit_cell.v;
            tile_size.h = ((tile_size.h + unit_cell.h - 1) / unit_cell.h) * unit_cell.h;
        }

        // But if that is larger than maximum tile size, round down to unit
        // cell size.
        if tile_size.v > max_tile_size.v {
            tile_size.v = (max_tile_size.v / unit_cell.v) * unit_cell.v;
        }

        if tile_size.h > max_tile_size.h {
            tile_size.h = (max_tile_size.h / unit_cell.h) * unit_cell.h;
        }

        if crate::dng_globals::get_print_timings() {
            println!("Render tile for below: {} x {}", tile_size.h, tile_size.v);
        }

        tile_size
    }

    /// `ProcessOnThread (threadIndex, area, tileSize, sniffer, progress)` —
    /// the exact C++ tile decomposition (nested RepeatingTile3/2/1 then
    /// tileSize iteration).
    fn process_on_thread(
        &mut self,
        thread_index: uint32,
        area: DngRect,
        tile_size: DngPoint,
        sniffer: Option<&Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
        progress: Option<&dyn DngAreaTaskProgress>,
    ) -> DngResult<()> {
        let mut repeating_tile1 = self.repeating_tile1();
        let mut repeating_tile2 = self.repeating_tile2();
        let mut repeating_tile3 = self.repeating_tile3();

        if repeating_tile1.is_empty() {
            repeating_tile1 = area;
        }
        if repeating_tile2.is_empty() {
            repeating_tile2 = area;
        }
        if repeating_tile3.is_empty() {
            repeating_tile3 = area;
        }

        // iter1 over repeatingTile3 ∩ area.
        let mut iter1 = crate::dng_tile_iterator::DngTileIterator::new_tile(repeating_tile3, area);
        let mut tile1 = DngRect::new();

        while iter1.get_one_tile(&mut tile1)? {
            let mut iter2 =
                crate::dng_tile_iterator::DngTileIterator::new_tile(repeating_tile2, tile1);
            let mut tile2 = DngRect::new();

            while iter2.get_one_tile(&mut tile2)? {
                let mut iter3 =
                    crate::dng_tile_iterator::DngTileIterator::new_tile(repeating_tile1, tile2);
                let mut tile3 = DngRect::new();

                while iter3.get_one_tile(&mut tile3)? {
                    let mut iter4 =
                        crate::dng_tile_iterator::DngTileIterator::new_tile_size(tile_size, tile3);
                    let mut tile4 = DngRect::new();

                    while iter4.get_one_tile(&mut tile4)? {
                        crate::dng_abort_sniffer::sniff_for_abort(sniffer)?;

                        self.process(thread_index, tile4, sniffer)?;

                        if let Some(progress) = progress {
                            progress.finished_tile(&tile4);
                        }
                    }
                }
            }
        }

        Ok(())
    }

}

/// Serial execution convenience mirroring `dng_area_task::Perform`
/// (single-threaded; the coordinator-mandated API shape).
pub fn process_on_single_thread(
    task: &mut dyn DngAreaTask,
    area: DngRect,
    sniffer: Option<&Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    progress: Option<&dyn DngAreaTaskProgress>,
) -> DngResult<()> {
    perform_area_task(task, area, sniffer, progress)
}

/// `dng_area_task::Perform (task, area, allocator, sniffer, progress)` —
/// serial dispatch (free function so `DngAreaTask` stays dyn-compatible).
pub fn perform_area_task(
    task: &mut dyn DngAreaTask,
    area: DngRect,
    sniffer: Option<&Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    progress: Option<&dyn DngAreaTaskProgress>,
) -> DngResult<()> {
    let tile_size = task.find_tile_size(area);

    task.start(1, area, tile_size, sniffer)?;

    task.process_on_thread(0, area, tile_size, sniffer, progress)?;

    task.finish(1);

    Ok(())
}

// ---------------------------------------------------------------------------
// dng_filter_task.
// ---------------------------------------------------------------------------

/// Port of `dng_filter_task` (abstract).  The C++ class holds raw image
/// references; the Rust port uses `ImageRef` handles (see DESIGN.md).
pub struct DngFilterTaskBase {
    pub name: DngString,
    pub src_image: crate::dng_image::ImageRef,
    pub dst_image: crate::dng_image::ImageRef,
    pub src_plane: uint32,
    pub src_planes: uint32,
    pub src_pixel_type: uint32,
    pub dst_plane: uint32,
    pub dst_planes: uint32,
    pub dst_pixel_type: uint32,
    pub src_repeat: DngPoint,
    pub src_tile_size: DngPoint,
    /// Overridable unit cell (used by dng_resample_task, C++ sets
    /// fUnitCell = (8,8) in the task constructor).
    pub unit_cell_override: DngPoint,
    /// Overridable max tile size (dng_resample_task scales it).
    pub max_tile_size_override: Option<DngPoint>,
}

impl DngFilterTaskBase {
    pub fn new(name: &str, src_image: crate::dng_image::ImageRef, dst_image: crate::dng_image::ImageRef) -> Self {
        let (src_planes, src_pixel_type) = {
            let src = src_image.read().unwrap();
            (src.planes(), src.pixel_type())
        };
        let (dst_planes, dst_pixel_type) = {
            let dst = dst_image.read().unwrap();
            (dst.planes(), dst.pixel_type())
        };

        DngFilterTaskBase {
            name: DngString::from_bytes(name.as_bytes()),
            src_image,
            dst_image,
            src_plane: 0,
            src_planes,
            src_pixel_type,
            dst_plane: 0,
            dst_planes,
            dst_pixel_type,
            src_repeat: DngPoint::new(1, 1),
            src_tile_size: DngPoint::new(0, 0),
            unit_cell_override: DngPoint::new(1, 1),
            max_tile_size_override: None,
        }
    }

    /// `SrcArea (dstArea)` — overridable hook.
    pub fn src_area(&self, dst_area: DngRect) -> DngRect {
        dst_area
    }

    /// `SrcTileSize (dstTileSize)`.
    pub fn src_tile_size(&self, dst_tile_size: DngPoint) -> DngPoint {
        self.src_area(DngRect::from_corners(0, 0, dst_tile_size.v, dst_tile_size.h))
            .size()
    }

    /// `Start` — allocate per-thread buffers is unnecessary in the serial
    /// port (buffers are allocated per call); records the tile size.
    pub fn start_filter(&mut self, dst_area: DngRect, tile_size: DngPoint) -> DngResult<()> {
        let _ = dst_area;
        self.src_tile_size = self.src_tile_size(tile_size);
        Ok(())
    }

    /// `Process (threadIndex, area, sniffer)` — the shared filter driver:
    /// get with edge repeat, call `process_area`, put back.
    pub fn process_filter(
        &mut self,
        thread_index: uint32,
        area: DngRect,
        process_area: &mut dyn FnMut(uint32, &mut DngPixelBuffer, &mut DngPixelBuffer) -> DngResult<()>,
    ) -> DngResult<()> {
        let src_area = self.src_area(area);

        let src_area_w = src_area.w() as int32;
        let src_area_h = src_area.h() as int32;

        if src_area_w > self.src_tile_size.h || src_area_h > self.src_tile_size.v {
            return Err(crate::dng_exceptions::memory_full()); // "Area exceeds tile size."
        }

        // Source buffer with edge-repeat padding.
        let mut src_buffer = DngPixelBuffer::with_planar_config(
            src_area,
            self.src_plane,
            self.src_planes,
            self.src_pixel_type,
            crate::dng_tag_values::pcRowInterleavedAlignSIMD,
            DngPixelBuffer::allocate_data_simd(src_area, self.src_planes, self.src_pixel_type)?,
        )?;

        {
            let src = self.src_image.read().unwrap();
            let src_repeat = self.src_repeat;
            src.get(
                &mut src_buffer,
                EdgeOption::EdgeRepeat,
                src_repeat.v as uint32,
                src_repeat.h as uint32,
            )?;
        }

        let mut dst_buffer = DngPixelBuffer::with_planar_config(
            area,
            self.dst_plane,
            self.dst_planes,
            self.dst_pixel_type,
            crate::dng_tag_values::pcRowInterleavedAlignSIMD,
            DngPixelBuffer::allocate_data_simd(area, self.dst_planes, self.dst_pixel_type)?,
        )?;

        process_area(thread_index, &mut src_buffer, &mut dst_buffer)?;

        let dst = self.dst_image.read().unwrap();
        dst.put(&dst_buffer)?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_range_parallel_task (serial).
// ---------------------------------------------------------------------------

/// Port of `dng_range_parallel_task` — serial index-range task.
pub struct DngRangeParallelTask<'a> {
    host_sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    start_index: int32,
    stop_index: int32,
    min_indices_per_thread: uint32,
    /// Serial callback invoked once with (threadIndex=0, startIndex, stopIndex).
    pub process_range: &'a mut dyn FnMut(uint32, int32, int32) -> DngResult<()>,
}

impl<'a> DngRangeParallelTask<'a> {
    /// `dng_range_parallel_task::info::info` defaults: minIndicesPerThread=1.
    pub fn new(
        start_index: int32,
        stop_index: int32,
        sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
        process_range: &'a mut dyn FnMut(uint32, int32, int32) -> DngResult<()>,
    ) -> Self {
        DngRangeParallelTask {
            host_sniffer: sniffer,
            start_index,
            stop_index,
            min_indices_per_thread: 1,
            process_range,
        }
    }

    /// `MinIndicesPerThread ()`.
    pub fn min_indices_per_thread(&self) -> uint32 {
        self.min_indices_per_thread
    }

    /// `Run ()` — serial.
    pub fn run(&mut self) -> DngResult<()> {
        (self.process_range)(0, self.start_index, self.stop_index)
    }
}

/// `dng_range_parallel_task::Do` — serial range execution.
pub fn range_parallel_do(
    _host: &mut crate::dng_host::DngHost,
    begin: int32,
    end: int32,
    task_name: &str,
    f: &mut dyn FnMut(uint32, int32, int32) -> DngResult<()>,
) -> DngResult<()> {
    let _ = task_name;
    f(0, begin, end)
}

#[allow(unused)]
fn unused_round(x: real64) -> int32 {
    round_int32_f64(x)
}

#[allow(unused)]
fn unused_max3(a: uint32, b: uint32, c: uint32) -> uint32 {
    max_uint32_3(a, b, c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_simple_image::DngSimpleImage;
    use crate::dng_tag_types::ttShort;

    struct ScaleByTwoTask {
        src: crate::dng_image::ImageRef,
        dst: crate::dng_image::ImageRef,
    }

    impl DngAreaTask for ScaleByTwoTask {
        fn name(&self) -> &str {
            "ScaleByTwoTask"
        }

        fn process(
            &mut self,
            _thread_index: uint32,
            tile: DngRect,
            _sniffer: Option<&Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
        ) -> DngResult<()> {
            let src = self.src.read().unwrap();
            let dst = self.dst.read().unwrap();

            let mut buffer = DngPixelBuffer::with_planar_config(
                tile,
                0,
                src.planes(),
                src.pixel_type(),
                crate::dng_tag_values::pcInterleaved,
                DngPixelBuffer::allocate_data(tile, src.planes(), src.pixel_type())?,
            )?;

            src.do_get(&mut buffer)?;

            let rows = tile.h();
            let cols = tile.w();
            for row in 0..rows as i64 {
                for col in 0..cols as i64 {
                    let v = buffer.const_pixel_u16(tile.t + row as int32, tile.l + col as int32, 0)?;
                    buffer.dirty_pixel_u16(tile.t + row as int32, tile.l + col as int32, 0, v * 2)?;
                }
            }

            dst.do_put(&buffer)?;
            Ok(())
        }
    }

    #[test]
    fn area_task_serial_execution() {
        let src = DngSimpleImage::new(DngRect::from_corners(0, 0, 8, 8), 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();
        let dst = DngSimpleImage::new(DngRect::from_corners(0, 0, 8, 8), 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();

        {
            let mut s = src.write().unwrap();
            s.set_constant_u16(3, s.bounds()).unwrap();
        }

        let mut task = ScaleByTwoTask {
            src: src.clone(),
            dst: dst.clone(),
        };

        crate::dng_area_task::process_on_single_thread(
            &mut task,
            DngRect::from_corners(0, 0, 8, 8),
            None,
            None,
        )
        .unwrap();

        let d = dst.read().unwrap();
        let mut buffer = DngPixelBuffer::with_planar_config(
            d.bounds(),
            0,
            1,
            ttShort,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(d.bounds(), 1, ttShort).unwrap(),
        )
        .unwrap();
        d.do_get(&mut buffer).unwrap();
        assert_eq!(buffer.const_pixel_u16(0, 0, 0).unwrap(), 6);
        assert_eq!(buffer.const_pixel_u16(7, 7, 0).unwrap(), 6);
    }

    #[test]
    fn find_tile_size_respects_max() {
        struct BigTask;
        impl DngAreaTask for BigTask {
            fn process(&mut self, _: uint32, _: DngRect, _: Option<&Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>) -> DngResult<()> {
                Ok(())
            }
            fn max_tile_size(&self) -> DngPoint {
                DngPoint::new(16, 16)
            }
        }

        let task = BigTask;
        let ts = task.find_tile_size(DngRect::from_corners(0, 0, 100, 100));
        assert!(ts.v <= 16 && ts.h <= 16);
    }

    #[test]
    fn range_parallel_serial() {
        let mut collected: Vec<(int32, int32)> = Vec::new();
        {
            let mut closure = |_thread: uint32, start: int32, stop: int32| {
                collected.push((start, stop));
                Ok(())
            };
            let mut task = DngRangeParallelTask::new(0, 10, None, &mut closure);
            task.run().unwrap();
        }
        assert_eq!(collected, vec![(0, 10)]);
    }
}
