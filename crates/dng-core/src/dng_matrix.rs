// Port of dng_matrix.h/.cpp — matrices (up to 4x4), vectors (up to 4),
// products, transpose, inverse (2x2/3x3 closed form + N-by-N Gauss-Jordan)
// and the pseudo-inverse with optional hint matrix.

use crate::dng_exceptions::matrix_math_error;
use crate::dng_errors::{DngError, DngResult};
use crate::dng_sdk_limits::kMaxColorPlanes;
use crate::dng_types::{real64, uint32};
use crate::dng_utils::{abs_real64, max_real64, min_real64, round_int32_f64};

const kNearZero: real64 = 1.0e-10;

/// Port of `dng_matrix` (up to kMaxColorPlanes x kMaxColorPlanes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DngMatrix {
    rows: uint32,
    cols: uint32,
    data: [[real64; kMaxColorPlanes as usize]; kMaxColorPlanes as usize],
}

impl Default for DngMatrix {
    fn default() -> Self {
        DngMatrix::new()
    }
}

impl DngMatrix {
    /// `dng_matrix ()` — empty.
    pub fn new() -> Self {
        DngMatrix {
            rows: 0,
            cols: 0,
            data: [[0.0; 4]; 4],
        }
    }

    /// `dng_matrix (rows, cols)` — zero-filled.
    pub fn with_dims(rows: uint32, cols: uint32) -> DngResult<Self> {
        if rows < 1 || rows > kMaxColorPlanes || cols < 1 || cols > kMaxColorPlanes {
            return Err(matrix_math_error());
        }
        Ok(DngMatrix {
            rows,
            cols,
            data: [[0.0; 4]; 4],
        })
    }

    pub fn from_rows(rows: uint32, cols: uint32, values: &[real64]) -> DngResult<Self> {
        let mut m = DngMatrix::with_dims(rows, cols)?;
        for j in 0..rows as usize {
            for k in 0..cols as usize {
                m.data[j][k] = values[j * cols as usize + k];
            }
        }
        Ok(m)
    }

    /// `Clear ()`.
    pub fn clear(&mut self) {
        self.rows = 0;
        self.cols = 0;
    }

    /// `SetIdentity (count)`.
    pub fn set_identity(&mut self, count: uint32) -> DngResult<()> {
        *self = DngMatrix::with_dims(count, count)?;
        for j in 0..count as usize {
            self.data[j][j] = 1.0;
        }
        Ok(())
    }

    pub fn rows(&self) -> uint32 {
        self.rows
    }

    pub fn cols(&self) -> uint32 {
        self.cols
    }

    /// `operator[]` read access: `m.get(row, col)`.
    #[inline]
    pub fn get(&self, row: usize, col: usize) -> real64 {
        self.data[row][col]
    }

    /// Mutable element access.
    #[inline]
    pub fn set(&mut self, row: usize, col: usize, value: real64) {
        self.data[row][col] = value;
    }

    /// Row slice access (read).
    #[inline]
    pub fn row(&self, row: usize) -> &[real64] {
        &self.data[row]
    }

    /// `IsEmpty ()`.
    pub fn is_empty(&self) -> bool {
        self.rows == 0 || self.cols == 0
    }

    /// `NotEmpty ()`.
    pub fn not_empty(&self) -> bool {
        !self.is_empty()
    }

    /// `IsDiagonal ()`.
    pub fn is_diagonal(&self) -> bool {
        if self.is_empty() || self.rows != self.cols {
            return false;
        }
        for j in 0..self.rows as usize {
            for k in 0..self.cols as usize {
                if j != k && self.data[j][k] != 0.0 {
                    return false;
                }
            }
        }
        true
    }

    /// `IsIdentity ()`.
    pub fn is_identity(&self) -> bool {
        if self.is_diagonal() {
            for j in 0..self.rows as usize {
                if self.data[j][j] != 1.0 {
                    return false;
                }
            }
            return true;
        }
        false
    }

    /// `MaxEntry ()`.
    pub fn max_entry(&self) -> real64 {
        if self.is_empty() {
            return 0.0;
        }
        let mut m = self.data[0][0];
        for j in 0..self.rows as usize {
            for k in 0..self.cols as usize {
                m = max_real64(m, self.data[j][k]);
            }
        }
        m
    }

    /// `MinEntry ()`.
    pub fn min_entry(&self) -> real64 {
        if self.is_empty() {
            return 0.0;
        }
        let mut m = self.data[0][0];
        for j in 0..self.rows as usize {
            for k in 0..self.cols as usize {
                m = min_real64(m, self.data[j][k]);
            }
        }
        m
    }

    /// `Scale (factor)`.
    pub fn scale(&mut self, factor: real64) {
        for j in 0..self.rows as usize {
            for k in 0..self.cols as usize {
                self.data[j][k] *= factor;
            }
        }
    }

    /// `Round (factor)`.
    pub fn round(&mut self, factor: real64) {
        let inv_factor = 1.0 / factor;
        for j in 0..self.rows as usize {
            for k in 0..self.cols as usize {
                self.data[j][k] = round_int32_f64(self.data[j][k] * factor) as real64 * inv_factor;
            }
        }
    }

    /// `SafeRound (factor)`.
    pub fn safe_round(&mut self, factor: real64) {
        let inv_factor = 1.0 / factor;
        for j in 0..self.rows as usize {
            // Round each row to the specified accuracy, but make sure the
            // rounding does not affect the total of the row more than needed.
            let mut error: real64 = 0.0;
            for k in 0..self.cols as usize {
                self.data[j][k] += error;
                let rounded = round_int32_f64(self.data[j][k] * factor) as real64 * inv_factor;
                error = self.data[j][k] - rounded;
                self.data[j][k] = rounded;
            }
        }
    }

    /// `AlmostEqual (m, slop)`.
    pub fn almost_equal(&self, m: &DngMatrix, slop: real64) -> bool {
        if self.rows != m.rows || self.cols != m.cols {
            return false;
        }
        for j in 0..self.rows as usize {
            for k in 0..self.cols as usize {
                if abs_real64(self.data[j][k] - m.data[j][k]) > slop {
                    return false;
                }
            }
        }
        true
    }

    /// `AlmostIdentity (slop)`.
    pub fn almost_identity(&self, slop: real64) -> bool {
        let mut m = DngMatrix::new();
        let _ = m.set_identity(self.rows);
        self.almost_equal(&m, slop)
    }

    /// `operator*` (matrix product).
    pub fn multiply(&self, b: &DngMatrix) -> DngResult<DngMatrix> {
        if self.cols != b.rows {
            return Err(matrix_math_error());
        }

        let mut c = DngMatrix::with_dims(self.rows, b.cols)?;

        for j in 0..c.rows as usize {
            for k in 0..c.cols as usize {
                let mut sum = 0.0;
                for m in 0..self.cols as usize {
                    sum += self.data[j][m] * b.data[m][k];
                }
                c.data[j][k] = sum;
            }
        }

        Ok(c)
    }

    /// `operator*` (matrix * vector).
    pub fn multiply_vector(&self, b: &DngVector) -> DngResult<DngVector> {
        if self.cols != b.count {
            return Err(matrix_math_error());
        }

        let mut c = DngVector::with_count(self.rows)?;

        for j in 0..c.count as usize {
            let mut sum = 0.0;
            for m in 0..self.cols as usize {
                sum += self.data[j][m] * b.data[m];
            }
            c.data[j] = sum;
        }

        Ok(c)
    }

    /// `operator+`.
    pub fn add(&self, b: &DngMatrix) -> DngResult<DngMatrix> {
        if self.cols != b.cols || self.rows != b.rows {
            return Err(matrix_math_error());
        }
        let mut c = *self;
        for j in 0..c.rows as usize {
            for k in 0..c.cols as usize {
                c.data[j][k] += b.data[j][k];
            }
        }
        Ok(c)
    }

    /// `Transpose (A)` as a method.
    pub fn transposed(&self) -> DngResult<DngMatrix> {
        let mut b = DngMatrix::with_dims(self.cols, self.rows)?;
        for j in 0..b.rows as usize {
            for k in 0..b.cols as usize {
                b.data[j][k] = self.data[k][j];
            }
        }
        Ok(b)
    }

    /// `Invert (A)` as a method.
    pub fn inverted(&self) -> DngResult<DngMatrix> {
        invert(self)
    }

    /// `Invert (A, hint)` as a method.
    pub fn inverted_with_hint(&self, hint: &DngMatrix) -> DngResult<DngMatrix> {
        invert_with_hint(self, hint)
    }
}

// ---------------------------------------------------------------------------
// Fixed-size convenience wrappers (dng_matrix_3by3, 4by3, 4by4).
// ---------------------------------------------------------------------------

/// Port of `dng_matrix_3by3`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DngMatrix3by3(pub DngMatrix);

impl DngMatrix3by3 {
    pub fn new(
        a00: real64, a01: real64, a02: real64,
        a10: real64, a11: real64, a12: real64,
        a20: real64, a21: real64, a22: real64,
    ) -> Self {
        DngMatrix3by3(
            DngMatrix::from_rows(
                3, 3,
                &[a00, a01, a02, a10, a11, a12, a20, a21, a22],
            )
            .unwrap(),
        )
    }

    /// Diagonal constructor `dng_matrix_3by3 (a00, a11, a22)`.
    pub fn diagonal(a00: real64, a11: real64, a22: real64) -> Self {
        let mut m = DngMatrix::with_dims(3, 3).unwrap();
        m.set(0, 0, a00);
        m.set(1, 1, a11);
        m.set(2, 2, a22);
        DngMatrix3by3(m)
    }

    pub fn identity() -> Self {
        let mut m = DngMatrix::with_dims(3, 3).unwrap();
        let _ = m.set_identity(3);
        DngMatrix3by3(m)
    }

    pub fn from_matrix(m: DngMatrix) -> DngResult<Self> {
        if m.rows() != 3 || m.cols() != 3 {
            return Err(matrix_math_error());
        }
        Ok(DngMatrix3by3(m))
    }

    pub fn inner(&self) -> &DngMatrix {
        &self.0
    }

    pub fn get(&self, row: usize, col: usize) -> real64 {
        self.0.get(row, col)
    }

    pub fn multiply_3by3(&self, other: &DngMatrix3by3) -> DngMatrix3by3 {
        DngMatrix3by3(self.0.multiply(&other.0).unwrap())
    }

    pub fn multiply_matrix(&self, other: &DngMatrix) -> DngResult<DngMatrix> {
        self.0.multiply(other)
    }

    pub fn inverted(&self) -> DngResult<DngMatrix3by3> {
        Ok(DngMatrix3by3(self.0.inverted()?))
    }
}

/// Port of `dng_matrix_4by3`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DngMatrix4by3(pub DngMatrix);

impl DngMatrix4by3 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        a00: real64, a01: real64, a02: real64,
        a10: real64, a11: real64, a12: real64,
        a20: real64, a21: real64, a22: real64,
        a30: real64, a31: real64, a32: real64,
    ) -> Self {
        DngMatrix4by3(
            DngMatrix::from_rows(
                4, 3,
                &[a00, a01, a02, a10, a11, a12, a20, a21, a22, a30, a31, a32],
            )
            .unwrap(),
        )
    }

    pub fn from_matrix(m: DngMatrix) -> DngResult<Self> {
        if m.rows() != 4 || m.cols() != 3 {
            return Err(matrix_math_error());
        }
        Ok(DngMatrix4by3(m))
    }

    pub fn inner(&self) -> &DngMatrix {
        &self.0
    }
}

/// Port of `dng_matrix_4by4`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DngMatrix4by4(pub DngMatrix);

impl DngMatrix4by4 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        a00: real64, a01: real64, a02: real64, a03: real64,
        a10: real64, a11: real64, a12: real64, a13: real64,
        a20: real64, a21: real64, a22: real64, a23: real64,
        a30: real64, a31: real64, a32: real64, a33: real64,
    ) -> Self {
        DngMatrix4by4(
            DngMatrix::from_rows(
                4, 4,
                &[
                    a00, a01, a02, a03, a10, a11, a12, a13, a20, a21, a22, a23, a30, a31, a32,
                    a33,
                ],
            )
            .unwrap(),
        )
    }

    /// Diagonal constructor.
    pub fn diagonal(a00: real64, a11: real64, a22: real64, a33: real64) -> Self {
        let mut m = DngMatrix::with_dims(4, 4).unwrap();
        m.set(0, 0, a00);
        m.set(1, 1, a11);
        m.set(2, 2, a22);
        m.set(3, 3, a33);
        DngMatrix4by4(m)
    }

    pub fn identity() -> Self {
        let mut m = DngMatrix::with_dims(4, 4).unwrap();
        let _ = m.set_identity(4);
        DngMatrix4by4(m)
    }

    /// `dng_matrix_4by4 (const dng_matrix &m)` — accepts 3x3 (padded) or 4x4.
    pub fn from_matrix(m: DngMatrix) -> DngResult<Self> {
        let is3by3 = m.rows() == 3 && m.cols() == 3;
        let is4by4 = m.rows() == 4 && m.cols() == 4;

        if !is3by3 && !is4by4 {
            return Err(matrix_math_error());
        }

        if is3by3 {
            let mut out = DngMatrix::with_dims(4, 4)?;
            for j in 0..3usize {
                for k in 0..3usize {
                    out.set(j, k, m.get(j, k));
                }
            }
            out.set(0, 3, 0.0);
            out.set(1, 3, 0.0);
            out.set(2, 3, 0.0);
            out.set(3, 0, 0.0);
            out.set(3, 1, 0.0);
            out.set(3, 2, 0.0);
            out.set(3, 3, 1.0);
            return Ok(DngMatrix4by4(out));
        }

        Ok(DngMatrix4by4(m))
    }

    pub fn inner(&self) -> &DngMatrix {
        &self.0
    }

    pub fn get(&self, row: usize, col: usize) -> real64 {
        self.0.get(row, col)
    }

    pub fn multiply_4by4(&self, other: &DngMatrix4by4) -> DngMatrix4by4 {
        DngMatrix4by4(self.0.multiply(&other.0).unwrap())
    }
}

// ---------------------------------------------------------------------------
// dng_vector.
// ---------------------------------------------------------------------------

/// Port of `dng_vector` (up to kMaxColorPlanes components).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DngVector {
    count: uint32,
    data: [real64; kMaxColorPlanes as usize],
}

impl Default for DngVector {
    fn default() -> Self {
        DngVector::new()
    }
}

impl DngVector {
    pub fn new() -> Self {
        DngVector {
            count: 0,
            data: [0.0; 4],
        }
    }

    pub fn with_count(count: uint32) -> DngResult<Self> {
        if count < 1 || count > kMaxColorPlanes {
            return Err(matrix_math_error());
        }
        Ok(DngVector {
            count,
            data: [0.0; 4],
        })
    }

    pub fn from_values(values: &[real64]) -> DngResult<Self> {
        let mut v = DngVector::with_count(values.len() as uint32)?;
        for (i, x) in values.iter().enumerate() {
            v.data[i] = *x;
        }
        Ok(v)
    }

    pub fn clear(&mut self) {
        self.count = 0;
    }

    /// `SetIdentity (count)` — all ones.
    pub fn set_identity(&mut self, count: uint32) -> DngResult<()> {
        *self = DngVector::with_count(count)?;
        for j in 0..count as usize {
            self.data[j] = 1.0;
        }
        Ok(())
    }

    pub fn count(&self) -> uint32 {
        self.count
    }

    #[inline]
    pub fn get(&self, index: usize) -> real64 {
        self.data[index]
    }

    #[inline]
    pub fn set(&mut self, index: usize, value: real64) {
        self.data[index] = value;
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn not_empty(&self) -> bool {
        !self.is_empty()
    }

    pub fn max_entry(&self) -> real64 {
        if self.is_empty() {
            return 0.0;
        }
        let mut m = self.data[0];
        for j in 0..self.count as usize {
            m = max_real64(m, self.data[j]);
        }
        m
    }

    pub fn min_entry(&self) -> real64 {
        if self.is_empty() {
            return 0.0;
        }
        let mut m = self.data[0];
        for j in 0..self.count as usize {
            m = min_real64(m, self.data[j]);
        }
        m
    }

    pub fn scale(&mut self, factor: real64) {
        for j in 0..self.count as usize {
            self.data[j] *= factor;
        }
    }

    pub fn round(&mut self, factor: real64) {
        let inv_factor = 1.0 / factor;
        for j in 0..self.count as usize {
            self.data[j] = round_int32_f64(self.data[j] * factor) as real64 * inv_factor;
        }
    }

    /// `AsDiagonal ()`.
    pub fn as_diagonal(&self) -> DngResult<DngMatrix> {
        let mut m = DngMatrix::with_dims(self.count, self.count)?;
        for j in 0..self.count as usize {
            m.set(j, j, self.data[j]);
        }
        Ok(m)
    }

    /// `AsColumn ()`.
    pub fn as_column(&self) -> DngResult<DngMatrix> {
        let mut m = DngMatrix::with_dims(self.count, 1)?;
        for j in 0..self.count as usize {
            m.set(j, 0, self.data[j]);
        }
        Ok(m)
    }

    pub fn subtract(&self, b: &DngVector) -> DngResult<DngVector> {
        let count = self.count;
        if count != b.count {
            return Err(DngError::Unsupported("Mismatch count in Dot".to_string()));
        }
        if count == 0 {
            return Ok(DngVector::new());
        }
        let mut result = DngVector::with_count(count)?;
        for i in 0..count as usize {
            result.data[i] = self.data[i] - b.data[i];
        }
        Ok(result)
    }
}

/// Port of `dng_vector_3`.
pub type DngVector3 = DngVector;

/// Port of `dng_vector_4`.
pub type DngVector4 = DngVector;

pub fn vector_3(a0: real64, a1: real64, a2: real64) -> DngResult<DngVector> {
    DngVector::from_values(&[a0, a1, a2])
}

pub fn vector_4(a0: real64, a1: real64, a2: real64, a3: real64) -> DngResult<DngVector> {
    DngVector::from_values(&[a0, a1, a2, a3])
}

// ---------------------------------------------------------------------------
// Free functions.
// ---------------------------------------------------------------------------

/// `Transpose (A)`.
pub fn transpose(a: &DngMatrix) -> DngResult<DngMatrix> {
    a.transposed()
}

fn invert2by2(a: &DngMatrix) -> DngResult<DngMatrix> {
    let a00 = a.get(0, 0);
    let a01 = a.get(0, 1);
    let a10 = a.get(1, 0);
    let a11 = a.get(1, 1);

    let det = a00 * a11 - a10 * a01;

    if abs_real64(det) < kNearZero {
        return Err(matrix_math_error());
    }

    let mut b = DngMatrix::with_dims(2, 2)?;
    b.set(0, 0, a11 / det);
    b.set(0, 1, -a01 / det);
    b.set(1, 0, -a10 / det);
    b.set(1, 1, a00 / det);
    Ok(b)
}

fn invert3by3(a: &DngMatrix) -> DngResult<DngMatrix> {
    let a00 = a.get(0, 0);
    let a01 = a.get(0, 1);
    let a02 = a.get(0, 2);
    let a10 = a.get(1, 0);
    let a11 = a.get(1, 1);
    let a12 = a.get(1, 2);
    let a20 = a.get(2, 0);
    let a21 = a.get(2, 1);
    let a22 = a.get(2, 2);

    let mut temp = [[0.0; 3]; 3];

    temp[0][0] = a11 * a22 - a21 * a12;
    temp[0][1] = a21 * a02 - a01 * a22;
    temp[0][2] = a01 * a12 - a11 * a02;
    temp[1][0] = a20 * a12 - a10 * a22;
    temp[1][1] = a00 * a22 - a20 * a02;
    temp[1][2] = a10 * a02 - a00 * a12;
    temp[2][0] = a10 * a21 - a20 * a11;
    temp[2][1] = a20 * a01 - a00 * a21;
    temp[2][2] = a00 * a11 - a10 * a01;

    let det = a00 * temp[0][0] + a01 * temp[1][0] + a02 * temp[2][0];

    if abs_real64(det) < kNearZero {
        return Err(matrix_math_error());
    }

    let mut b = DngMatrix::with_dims(3, 3)?;
    for j in 0..3usize {
        for k in 0..3usize {
            b.set(j, k, temp[j][k] / det);
        }
    }
    Ok(b)
}

fn invert_nbyn(a: &DngMatrix) -> DngResult<DngMatrix> {
    let n = a.rows() as usize;
    let augmented_cols = 2 * n;

    let mut temp = [[0.0; kMaxColorPlanes as usize * 2]; kMaxColorPlanes as usize];

    for i in 0..n {
        for j in 0..n {
            temp[i][j] = a.get(i, j);
            temp[i][j + n] = if i == j { 1.0 } else { 0.0 };
        }
    }

    for i in 0..n {
        // Find row iMax with largest absolute entry in column i.
        let mut i_max = i;
        let mut v_max: real64 = -1.0;

        for k in i..n {
            let v = abs_real64(a.get(k, i));
            if v > v_max {
                v_max = v;
                i_max = k;
            }
        }

        let alpha = temp[i_max][i];

        if abs_real64(alpha) < kNearZero {
            return Err(matrix_math_error());
        }

        // Swap rows i and iMax, column by column.
        if i != i_max {
            temp.swap(i, i_max);
        }

        for j in 0..augmented_cols {
            temp[i][j] /= alpha;
        }

        for k in 0..n {
            if i != k {
                let beta = temp[k][i];
                for j in 0..augmented_cols {
                    temp[k][j] -= beta * temp[i][j];
                }
            }
        }
    }

    let mut b = DngMatrix::with_dims(n as uint32, n as uint32)?;
    for i in 0..n {
        for j in 0..n {
            b.set(i, j, temp[i][j + n]);
        }
    }
    Ok(b)
}

/// `Invert (A)`.
pub fn invert(a: &DngMatrix) -> DngResult<DngMatrix> {
    if a.rows() < 2 || a.cols() < 2 {
        return Err(matrix_math_error());
    }

    if a.rows() == a.cols() {
        if a.rows() == 3 {
            return invert3by3(a);
        } else if a.rows() == 2 {
            return invert2by2(a);
        }
        return invert_nbyn(a);
    }

    // Compute the pseudo inverse.
    let b = a.transposed()?;
    Ok(invert(&b.multiply(a)?)?.multiply(&b)?)
}

/// `Invert (A, hint)`.
pub fn invert_with_hint(a: &DngMatrix, hint: &DngMatrix) -> DngResult<DngMatrix> {
    if a.rows() == a.cols() || a.rows() != hint.cols() || a.cols() != hint.rows() {
        invert(a)
    } else {
        // Use the specified hint matrix.
        Ok(invert(&hint.multiply(a)?)?.multiply(hint)?)
    }
}

/// `Dot (a, b)`.
pub fn dot(a: &DngVector, b: &DngVector) -> real64 {
    debug_assert!(a.count() == b.count(), "Cannot take dot product between vectors of different size.");

    let mut sum = 0.0;
    for j in 0..a.count() as usize {
        sum += a.get(j) * b.get(j);
    }
    sum
}

/// `Distance (a, b)`.
pub fn distance(a: &DngVector, b: &DngVector) -> real64 {
    let c = a.subtract(b).unwrap();
    dot(&c, &c).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: real64, b: real64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn identity_and_diagonal() {
        let mut m = DngMatrix::new();
        m.set_identity(3).unwrap();
        assert!(m.is_identity());
        assert!(m.is_diagonal());
        assert_eq!(m.rows(), 3);
        assert_eq!(m.get(1, 1), 1.0);

        let d = DngMatrix3by3::diagonal(2.0, 3.0, 4.0);
        assert!(d.0.is_diagonal());
        assert!(!d.0.is_identity());
    }

    #[test]
    fn multiply_hand_computed() {
        // [[1,2],[3,4]] * [[5,6],[7,8]] = [[19,22],[43,50]]
        let a = DngMatrix::from_rows(2, 2, &[1.0, 2.0, 3.0, 4.0]).unwrap();
        let b = DngMatrix::from_rows(2, 2, &[5.0, 6.0, 7.0, 8.0]).unwrap();
        let c = a.multiply(&b).unwrap();
        assert_eq!(c.get(0, 0), 19.0);
        assert_eq!(c.get(0, 1), 22.0);
        assert_eq!(c.get(1, 0), 43.0);
        assert_eq!(c.get(1, 1), 50.0);

        // Dimension mismatch errors.
        let d = DngMatrix::from_rows(3, 3, &[1.0; 9]).unwrap();
        assert!(a.multiply(&d).is_err());
    }

    #[test]
    fn matrix_vector_product() {
        // [[1,2,3],[4,5,6],[7,8,9]] * [1,0,2] = [7,16,25]
        let a = DngMatrix::from_rows(3, 3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]).unwrap();
        let v = DngVector::from_values(&[1.0, 0.0, 2.0]).unwrap();
        let r = a.multiply_vector(&v).unwrap();
        assert_eq!(r.get(0), 7.0);
        assert_eq!(r.get(1), 16.0);
        assert_eq!(r.get(2), 25.0);
    }

    #[test]
    fn invert_3by3() {
        // Rotation-free example: [[2,0,0],[0,4,0],[0,0,8]]
        let a = DngMatrix3by3::diagonal(2.0, 4.0, 8.0);
        let inv = a.0.inverted().unwrap();
        assert!(close(inv.get(0, 0), 0.5));
        assert!(close(inv.get(1, 1), 0.25));
        assert!(close(inv.get(2, 2), 0.125));

        // A * A^-1 == I for a general matrix.
        let m = DngMatrix::from_rows(3, 3, &[1.0, 2.0, 0.0, 3.0, 4.0, 5.0, 0.0, 6.0, 7.0]).unwrap();
        let inv = m.inverted().unwrap();
        let product = m.multiply(&inv).unwrap();
        assert!(product.almost_identity(1e-9));

        // Singular matrix errors.
        let sing = DngMatrix::from_rows(3, 3, &[1.0, 2.0, 3.0, 2.0, 4.0, 6.0, 1.0, 1.0, 1.0]).unwrap();
        assert!(sing.inverted().is_err());
    }

    #[test]
    fn invert_2by2_and_pseudo() {
        let a = DngMatrix::from_rows(2, 2, &[4.0, 7.0, 2.0, 6.0]).unwrap();
        let inv = a.inverted().unwrap();
        // det = 10 -> [[0.6, -0.7], [-0.2, 0.4]]
        assert!(close(inv.get(0, 0), 0.6));
        assert!(close(inv.get(0, 1), -0.7));
        assert!(close(inv.get(1, 0), -0.2));
        assert!(close(inv.get(1, 1), 0.4));

        // Pseudo-inverse of a tall 3x2 matrix (full column rank; the C++
        // Invert pseudo path requires Transpose(A)*A to be non-singular).
        let a = DngMatrix::from_rows(3, 2, &[1.0, 0.0, 0.0, 1.0, 1.0, 1.0]).unwrap();
        let pinv = a.inverted().unwrap();
        assert_eq!(pinv.rows(), 2);
        assert_eq!(pinv.cols(), 3);
        // Verify A * pinv * A == A.
        let check = a.multiply(&pinv).unwrap().multiply(&a).unwrap();
        assert!(check.almost_equal(&a, 1e-9));
    }

    #[test]
    fn invert_with_hint() {
        // 3x2 matrix with 2x3 hint.
        let a = DngMatrix::from_rows(3, 2, &[1.0, 0.0, 0.0, 1.0, 1.0, 1.0]).unwrap();
        let hint = a.transposed().unwrap();
        let pinv = a.inverted_with_hint(&hint).unwrap();
        assert_eq!(pinv.rows(), 2);
        assert_eq!(pinv.cols(), 3);
    }

    #[test]
    fn transpose_scale_round() {
        let a = DngMatrix::from_rows(2, 3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let t = a.transposed().unwrap();
        assert_eq!(t.rows(), 3);
        assert_eq!(t.cols(), 2);
        assert_eq!(t.get(2, 0), 3.0);

        let mut b = a;
        b.scale(2.0);
        assert_eq!(b.get(1, 2), 12.0);

        b.round(1000.0);
        assert_eq!(b.get(1, 2), 12.0);

        let mut c = DngMatrix::from_rows(2, 3, &[0.123456, 0.223456, 0.323456, 0.423456, 0.523456, 0.623456]).unwrap();
        c.safe_round(1000.0);
        assert_eq!(c.get(0, 0), 0.123);
    }

    #[test]
    fn vector_ops() {
        let a = DngVector::from_values(&[1.0, 2.0, 3.0]).unwrap();
        let b = DngVector::from_values(&[4.0, 5.0, 6.0]).unwrap();
        assert_eq!(dot(&a, &b), 32.0);
        assert!((distance(&a, &b) - 3.0f64.sqrt().powi(2) * 1.0).abs() < 1e-12
            || (distance(&a, &b) - 27.0f64.sqrt()).abs() < 1e-12);

        let diag = a.as_diagonal().unwrap();
        assert_eq!(diag.rows(), 3);
        assert_eq!(diag.get(1, 1), 2.0);

        let col = a.as_column().unwrap();
        assert_eq!(col.cols(), 1);
        assert_eq!(col.get(2, 0), 3.0);

        let mut v = DngVector::from_values(&[0.12, 0.26, 0.34]).unwrap();
        v.round(10.0); // nearest 0.1
        assert!(close(v.get(0), 0.1));
    }

    #[test]
    fn matrix_4by4_and_padding() {
        let m3 = DngMatrix::from_rows(3, 3, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
        let m4 = DngMatrix4by4::from_matrix(m3).unwrap();
        assert_eq!(m4.get(3, 3), 1.0);
        assert_eq!(m4.get(0, 3), 0.0);

        let a = DngMatrix4by4::new(
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
        );
        let i = DngMatrix4by4::identity();
        let p = a.multiply_4by4(&i);
        assert_eq!(p.get(1, 1), 6.0);
        assert_eq!(p.get(3, 3), 16.0);
    }

    #[test]
    fn vector_count_validation() {
        assert!(DngVector::with_count(0).is_err());
        assert!(DngVector::with_count(5).is_err());
        assert!(DngVector::with_count(4).is_ok());
        assert!(DngMatrix::with_dims(5, 3).is_err());
        assert!(DngMatrix::with_dims(0, 3).is_err());
    }
}
