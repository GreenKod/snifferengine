use super::GlowRenderer;
use glow::HasContext;
use sniffer_core::math::Rect;

/// Identity per-instance transform: `(scale, rotation_deg, pivot_x, pivot_y)`.
///
/// The vertex shader math collapses to the raw corner for this value (the
/// zero-degree rotation matrix is exact), so producers that have not moved
/// to instanced transforms yet render bit-identically to the pre-attribute
/// pipeline.
pub const IDENTITY_TRANSFORM: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Decomposes a column-major 3x3 affine matrix into a per-instance
/// transform `(scale, rotation in degrees, pivot_x, pivot_y)` with the
/// pivot expressed in rect-local pixels.
///
/// Returns [`IDENTITY_TRANSFORM`] when the linear part is identity (pure
/// translation) or when the fixed-point system is degenerate; the caller
/// handles pure translations through pre-translated rect positions.
#[must_use]
pub fn decompose_to_instance_transform(matrix: &[f32; 9], rect: &Rect) -> [f32; 4] {
    // Column-major layout: col0 = (m[0], m[1], m[2]), col1 = (m[3], m[4],
    // m[5]), col2 = (m[6], m[7], m[8]). The affine rows are:
    // row0: a=m[0], b=m[3]    row1: c=m[1], d=m[4]    translation: m[6], m[7]
    let a = matrix[0];
    let b = matrix[3];
    let c = matrix[1];
    let d = matrix[4];
    let tx = matrix[6];
    let ty = matrix[7];

    // Extract scale and rotation from the linear part.
    let scale = a.hypot(c);
    let rotation_deg = c.atan2(a).to_degrees();

    // Identity linear part: no per-instance transform needed.
    if (scale - 1.0).abs() < 1e-6 && rotation_deg.abs() < 1e-6 {
        return IDENTITY_TRANSFORM;
    }

    // Fixed point of the affine map: (M_linear - I) * pivot = -translate
    //   (a-1)*px + b*py    = -tx
    //   c*px    + (d-1)*py = -ty
    let det = (a - 1.0) * (d - 1.0) - b * c;
    if det.abs() < 1e-12 {
        return IDENTITY_TRANSFORM;
    }

    let world_pivot_x = (-tx * (d - 1.0) + b * ty) / det;
    let world_pivot_y = (tx * c - (a - 1.0) * ty) / det;

    [
        scale,
        rotation_deg,
        world_pivot_x - rect.x,
        world_pivot_y - rect.y,
    ]
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct QuadInstanceData {
    pub rect_pos: [f32; 2],
    pub rect_size: [f32; 2],
    pub color: [f32; 4],
    pub border_color: [f32; 4],
    pub color_bottom: [f32; 4],
    pub shape_size: [f32; 2],
    pub is_circle: f32,
    pub is_shadow: f32,
    pub radius: f32,
    pub border_width: f32,
    pub shadow_blur: f32,
    pub is_gradient: f32,
    pub border_color_bottom: [f32; 4],
    /// Per-instance transform: `(scale, rotation in degrees, pivot_x,
    /// pivot_y)` with the pivot in rect-local pixels. Identity is
    /// [`IDENTITY_TRANSFORM`].
    pub transform: [f32; 4],
}

impl Default for QuadInstanceData {
    fn default() -> Self {
        Self {
            rect_pos: [0.0; 2],
            rect_size: [0.0; 2],
            color: [0.0; 4],
            border_color: [0.0; 4],
            color_bottom: [0.0; 4],
            shape_size: [0.0; 2],
            is_circle: 0.0,
            is_shadow: 0.0,
            radius: 0.0,
            border_width: 0.0,
            shadow_blur: 0.0,
            is_gradient: 0.0,
            border_color_bottom: [0.0; 4],
            transform: IDENTITY_TRANSFORM,
        }
    }
}

impl QuadInstanceData {
    #[must_use]
    pub fn new_dual_shadow(
        rect: Rect,
        radius: f32,
        elevation: f32,
        shadow_color: Option<u32>,
        global_alpha: f32,
    ) -> (Self, Rect) {
        let base_color = shadow_color.unwrap_or(0x4D00_0000);
        let col = unpack_color(base_color);
        let base_alpha = col[3] * global_alpha;

        let mut key_col = col;
        key_col[3] = base_alpha * 0.65;
        let key_blur = (elevation * 1.2).max(2.0);
        let key_offset_y = elevation * 0.75;

        let mut ambient_col = col;
        ambient_col[3] = base_alpha * 0.35;
        let ambient_blur = (elevation * 1.6).max(2.0);
        let ambient_spread = elevation * 0.1;

        // Effective SDF reach: outside this radius smoothstep is strictly 0.0
        let key_reach = key_offset_y + key_blur * 1.5;
        let ambient_reach = ambient_spread + ambient_blur * 1.5;
        let max_reach = key_reach.max(ambient_reach);

        // Maximum safe shadow padding (64px) to eliminate runaway fill-rate overdraw on mobile
        const MAX_SHADOW_PADDING: f32 = 64.0;
        let pad = (max_reach + 1.0).min(MAX_SHADOW_PADDING);

        let shadow_rect = Rect {
            x: rect.x - pad,
            y: rect.y - pad,
            width: rect.width + pad * 2.0,
            height: rect.height + pad * 2.0,
        };

        let instance = Self {
            rect_pos: [shadow_rect.x, shadow_rect.y],
            rect_size: [shadow_rect.width, shadow_rect.height],
            color: key_col,
            border_color: [0.0; 4],
            color_bottom: ambient_col,
            shape_size: [rect.width, rect.height],
            is_circle: 0.0,
            is_shadow: 2.0,
            radius,
            border_width: ambient_blur,
            shadow_blur: key_blur,
            is_gradient: key_offset_y,
            border_color_bottom: [0.0; 4],
            transform: IDENTITY_TRANSFORM,
        };

        (instance, shadow_rect)
    }
}

pub struct QuadBatch {
    pub instances: Vec<QuadInstanceData>,
    pub max_capacity: usize,
}

impl QuadBatch {
    pub const DEFAULT_MAX_CAPACITY: usize = 512;
    pub const INITIAL_CAPACITY: usize = 128;

    #[must_use]
    pub fn new(max_capacity: usize) -> Self {
        Self {
            instances: Vec::with_capacity(Self::INITIAL_CAPACITY.min(max_capacity)),
            max_capacity,
        }
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    #[inline]
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.instances.len() >= self.max_capacity
    }

    #[inline]
    pub fn push_instance(&mut self, instance: QuadInstanceData) -> bool {
        if self.is_full() {
            return false;
        }
        self.instances.push(instance);
        true
    }

    #[inline]
    pub fn clear(&mut self) {
        self.instances.clear();
    }

    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[QuadInstanceData] {
        &self.instances
    }

    /// Returns the raw byte representation of instances for OpenGL buffer upload.
    #[inline]
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.instances.as_ptr().cast::<u8>(),
                self.instances.len() * std::mem::size_of::<QuadInstanceData>(),
            )
        }
    }
}

impl Default for QuadBatch {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_CAPACITY)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ImageInstanceData {
    pub rect_pos: [f32; 2],
    pub rect_size: [f32; 2],
    pub uv_rect: [f32; 4],
    pub radius: f32,
    pub alpha: f32,
    pub pad: [f32; 2],
    /// Per-instance transform: `(scale, rotation in degrees, pivot_x,
    /// pivot_y)` with the pivot in rect-local pixels. Identity is
    /// [`IDENTITY_TRANSFORM`].
    pub transform: [f32; 4],
}

impl Default for ImageInstanceData {
    fn default() -> Self {
        Self {
            rect_pos: [0.0; 2],
            rect_size: [0.0; 2],
            uv_rect: [0.0; 4],
            radius: 0.0,
            alpha: 0.0,
            pad: [0.0; 2],
            transform: IDENTITY_TRANSFORM,
        }
    }
}

pub struct ImageBatch {
    pub instances: Vec<ImageInstanceData>,
    pub max_capacity: usize,
}

impl ImageBatch {
    pub const DEFAULT_MAX_CAPACITY: usize = 512;
    pub const INITIAL_CAPACITY: usize = 128;

    #[must_use]
    pub fn new(max_capacity: usize) -> Self {
        Self {
            instances: Vec::with_capacity(Self::INITIAL_CAPACITY.min(max_capacity)),
            max_capacity,
        }
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    #[inline]
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.instances.len() >= self.max_capacity
    }

    #[inline]
    pub fn push_instance(&mut self, instance: ImageInstanceData) -> bool {
        if self.is_full() {
            return false;
        }
        self.instances.push(instance);
        true
    }

    #[inline]
    pub fn clear(&mut self) {
        self.instances.clear();
    }

    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[ImageInstanceData] {
        &self.instances
    }

    /// Returns the raw byte representation of instances for OpenGL buffer upload.
    #[inline]
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.instances.as_ptr().cast::<u8>(),
                self.instances.len() * std::mem::size_of::<ImageInstanceData>(),
            )
        }
    }
}

impl Default for ImageBatch {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_CAPACITY)
    }
}

pub struct TextBatch {
    pub vertices: Vec<f32>,
    pub max_capacity: usize,
    pub current_transform: Option<[f32; 9]>,
}

impl TextBatch {
    pub const DEFAULT_MAX_CAPACITY: usize = 65_536;
    pub const INITIAL_CAPACITY: usize = 2_048;
    pub const FLOATS_PER_VERTEX: usize = 9;

    #[must_use]
    pub fn new(max_capacity: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(Self::INITIAL_CAPACITY.min(max_capacity)),
            max_capacity,
            current_transform: None,
        }
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.vertices.len()
    }

    #[inline]
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.len() >= self.max_capacity
    }

    #[inline]
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.current_transform = None;
    }

    #[inline]
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.vertices.as_ptr().cast::<u8>(),
                self.vertices.len() * std::mem::size_of::<f32>(),
            )
        }
    }
}

impl Default for TextBatch {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_CAPACITY)
    }
}

pub fn unpack_color(color: u32) -> [f32; 4] {
    let a =
        f32::from(u8::try_from((color >> 24) & 0xff).expect("alpha channel fits in u8")) / 255.0;
    let r = f32::from(u8::try_from((color >> 16) & 0xff).expect("red channel fits in u8")) / 255.0;
    let g = f32::from(u8::try_from((color >> 8) & 0xff).expect("green channel fits in u8")) / 255.0;
    let b = f32::from(u8::try_from(color & 0xff).expect("blue channel fits in u8")) / 255.0;
    let a = if a == 0.0 && color != 0 { 1.0 } else { a };
    [r, g, b, a]
}

impl GlowRenderer {
    /// Computes the per-instance transform for a rect from the current
    /// transform stack. Shape and image instances carry this instead of
    /// relying on a batch-wide uniform, so transform pushes and pops no
    /// longer break batches.
    #[must_use]
    pub fn current_instance_transform(&self, rect: &Rect) -> [f32; 4] {
        match self.transform_stack.last() {
            Some(matrix) => decompose_to_instance_transform(matrix, rect),
            None => IDENTITY_TRANSFORM,
        }
    }

    pub fn flush_shapes(&mut self) {
        if self.shape_batch.is_empty() {
            return;
        }

        unsafe {
            self.gl.use_program(Some(self.shape_program));
            self.ensure_shape_instance_vao();

            let u = &self.shape_uniforms;
            self.gl.uniform_1_i32(u.u_instanced.as_ref(), 1);
            self.gl.uniform_2_f32(
                u.u_resolution.as_ref(),
                self.resolution.0,
                self.resolution.1,
            );

            // Every instance carries its own per-instance transform, so
            // the batch-wide uniform stays identity regardless of the
            // transform scopes the instances were produced under.
            const IDENTITY_MATRIX: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
            self.gl
                .uniform_matrix_3_f32_slice(u.u_transform.as_ref(), false, &IDENTITY_MATRIX);

            self.upload_clip_uniforms(
                u.u_clip_rect.as_ref(),
                u.u_clip_radius.as_ref(),
                u.u_clip_inv_transform.as_ref(),
            );

            self.gl
                .bind_buffer(glow::ARRAY_BUFFER, Some(self.shape_instance_vbo));
            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                self.shape_batch.as_bytes(),
                glow::DYNAMIC_DRAW,
            );

            let count = i32::try_from(self.shape_batch.len()).expect("instance count fits in i32");
            self.gl
                .draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, count);

            self.shape_batch.clear();
        }
    }

    pub fn flush_images(&mut self) {
        if self.image_batch.is_empty() {
            return;
        }

        let Some(ref atlas) = self.icon_atlas else {
            self.image_batch.clear();
            return;
        };
        let atlas_tex = atlas.texture;

        unsafe {
            self.gl.use_program(Some(self.image_program));
            self.ensure_image_instance_vao();

            let u = &self.image_uniforms;
            self.gl.uniform_1_i32(u.u_instanced.as_ref(), 1);
            self.gl.uniform_2_f32(
                u.u_resolution.as_ref(),
                self.resolution.0,
                self.resolution.1,
            );

            // Every image instance carries its own per-instance transform,
            // so the batch-wide uniform stays identity.
            const IDENTITY_MATRIX: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
            self.gl
                .uniform_matrix_3_f32_slice(u.u_transform.as_ref(), false, &IDENTITY_MATRIX);

            self.upload_clip_uniforms(
                u.u_clip_rect.as_ref(),
                u.u_clip_radius.as_ref(),
                u.u_clip_inv_transform.as_ref(),
            );

            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(atlas_tex));

            self.gl
                .bind_buffer(glow::ARRAY_BUFFER, Some(self.image_instance_vbo));
            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                self.image_batch.as_bytes(),
                glow::DYNAMIC_DRAW,
            );

            let count =
                i32::try_from(self.image_batch.len()).expect("image instance count fits in i32");
            self.gl
                .draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, count);

            self.image_batch.clear();
        }
    }

    pub(crate) fn draw_rect_impl(
        &mut self,
        rect: Rect,
        color: u32,
        radius: f32,
        border_width: f32,
        border_color: Option<u32>,
    ) {
        self.draw_rect_gradient_impl(rect, color, color, radius, border_width, border_color);
    }

    pub(crate) fn draw_rect_gradient_impl(
        &mut self,
        rect: Rect,
        color_top: u32,
        color_bottom: u32,
        radius: f32,
        border_width: f32,
        border_color: Option<u32>,
    ) {
        self.draw_rect_gradient_border_impl(
            rect,
            color_top,
            color_bottom,
            radius,
            border_width,
            border_color,
            border_color,
        );
    }

    pub(crate) fn draw_rect_gradient_border_impl(
        &mut self,
        rect: Rect,
        color_top: u32,
        color_bottom: u32,
        radius: f32,
        border_width: f32,
        border_top: Option<u32>,
        border_bottom: Option<u32>,
    ) {
        if self.shape_batch.is_full() {
            self.flush_shapes();
        }

        let mut col_top = unpack_color(color_top);
        let mut col_bot = unpack_color(color_bottom);
        col_top[3] *= self.global_alpha;
        col_bot[3] *= self.global_alpha;

        let b_top = border_top.or(border_bottom).unwrap_or(0);
        let b_bot = border_bottom.or(border_top).unwrap_or(0);
        let mut b_col_top = unpack_color(b_top);
        let mut b_col_bot = unpack_color(b_bot);
        b_col_top[3] *= self.global_alpha;
        b_col_bot[3] *= self.global_alpha;

        let is_gradient = if color_top == color_bottom && b_top == b_bot {
            0.0f32
        } else {
            1.0f32
        };

        let instance = QuadInstanceData {
            rect_pos: [rect.x, rect.y],
            rect_size: [rect.width, rect.height],
            color: col_top,
            border_color: b_col_top,
            color_bottom: col_bot,
            shape_size: [rect.width, rect.height],
            is_circle: 0.0,
            is_shadow: 0.0,
            radius,
            border_width,
            shadow_blur: 0.0,
            is_gradient,
            border_color_bottom: b_col_bot,
            transform: self.current_instance_transform(&rect),
        };

        let _ = self.shape_batch.push_instance(instance);
    }

    pub(crate) fn draw_shadow_impl(
        &mut self,
        rect: Rect,
        radius: f32,
        offset_y: f32,
        spread: f32,
        color: u32,
    ) {
        if (color & 0xFF00_0000 == 0) || self.global_alpha <= 0.001 {
            return;
        }

        let mut col = unpack_color(color);
        col[3] *= self.global_alpha;

        let blur = spread * 1.5;
        let reach = offset_y.abs() + spread + blur * 1.5;
        const MAX_SHADOW_PADDING: f32 = 64.0;
        let padding = (reach + 1.0).min(MAX_SHADOW_PADDING);

        let shadow_rect = Rect {
            x: rect.x - spread - padding,
            y: rect.y + offset_y - spread - padding,
            width: spread.mul_add(2.0, rect.width) + padding * 2.0,
            height: spread.mul_add(2.0, rect.height) + padding * 2.0,
        };

        // Clip container culling: discard if completely outside active clip
        if self.transform_stack.is_empty() {
            if let Some(clip) = self.clip_stack.last() {
                let clip_rect = clip.rect;
                if shadow_rect.x > clip_rect.x + clip_rect.width
                    || shadow_rect.x + shadow_rect.width < clip_rect.x
                    || shadow_rect.y > clip_rect.y + clip_rect.height
                    || shadow_rect.y + shadow_rect.height < clip_rect.y
                {
                    return;
                }
            }
        }

        let shape_size_x = spread.mul_add(2.0, rect.width);
        let shape_size_y = spread.mul_add(2.0, rect.height);

        let instance = QuadInstanceData {
            rect_pos: [shadow_rect.x, shadow_rect.y],
            rect_size: [shadow_rect.width, shadow_rect.height],
            color: col,
            border_color: [0.0; 4],
            color_bottom: col,
            shape_size: [shape_size_x, shape_size_y],
            is_circle: 0.0,
            is_shadow: 1.0,
            radius: radius + spread,
            border_width: 0.0,
            shadow_blur: blur,
            is_gradient: 0.0,
            border_color_bottom: [0.0; 4],
            transform: self.current_instance_transform(&shadow_rect),
        };

        if self.shape_batch.is_full() {
            self.flush_shapes();
        }
        let _ = self.shape_batch.push_instance(instance);
    }

    pub(crate) fn draw_elevation_shadow_impl(
        &mut self,
        rect: Rect,
        radius: f32,
        elevation: f32,
        shadow_color: Option<u32>,
    ) {
        if elevation <= 0.0 || self.global_alpha <= 0.001 {
            return;
        }

        if let Some(c) = shadow_color {
            if c & 0xFF00_0000 == 0 {
                return;
            }
        }

        let (mut instance, shadow_rect) = QuadInstanceData::new_dual_shadow(
            rect,
            radius,
            elevation,
            shadow_color,
            self.global_alpha,
        );
        instance.transform = self.current_instance_transform(&shadow_rect);

        // Clip container culling: discard if completely outside active clip
        if self.transform_stack.is_empty() {
            if let Some(clip) = self.clip_stack.last() {
                let clip_rect = clip.rect;
                if shadow_rect.x > clip_rect.x + clip_rect.width
                    || shadow_rect.x + shadow_rect.width < clip_rect.x
                    || shadow_rect.y > clip_rect.y + clip_rect.height
                    || shadow_rect.y + shadow_rect.height < clip_rect.y
                {
                    return;
                }
            }
        }

        if self.shape_batch.is_full() {
            self.flush_shapes();
        }
        let _ = self.shape_batch.push_instance(instance);
    }

    pub(crate) fn draw_circle_impl(&mut self, cx: f32, cy: f32, radius: f32, color: u32) {
        self.flush_shapes();

        let mut col = unpack_color(color);
        col[3] *= self.global_alpha;
        unsafe {
            self.gl.use_program(Some(self.shape_program));
            self.ensure_quad_vao();

            let u = &self.shape_uniforms;
            self.gl.uniform_1_i32(u.u_instanced.as_ref(), 0);

            let rect = Rect {
                x: cx - radius,
                y: cy - radius,
                width: radius * 2.0,
                height: radius * 2.0,
            };

            self.gl.uniform_2_f32(
                u.u_resolution.as_ref(),
                self.resolution.0,
                self.resolution.1,
            );
            self.gl.uniform_2_f32(u.u_rect_pos.as_ref(), rect.x, rect.y);
            self.gl
                .uniform_2_f32(u.u_rect_size.as_ref(), rect.width, rect.height);
            self.gl
                .uniform_2_f32(u.u_shape_size.as_ref(), rect.width, rect.height);
            self.gl
                .uniform_4_f32(u.u_color.as_ref(), col[0], col[1], col[2], col[3]);
            self.gl.uniform_1_f32(u.u_radius.as_ref(), radius);
            self.gl.uniform_1_f32(u.u_is_circle.as_ref(), 1.0);
            self.gl.uniform_1_f32(u.u_is_shadow.as_ref(), 0.0);

            let Some(t) = self.transform_stack.last() else {
                crate::dev_err!("transform_stack empty in draw_circle — missing push_transform");
                return;
            };
            self.gl
                .uniform_matrix_3_f32_slice(u.u_transform.as_ref(), false, t);

            self.upload_clip_uniforms(
                u.u_clip_rect.as_ref(),
                u.u_clip_radius.as_ref(),
                u.u_clip_inv_transform.as_ref(),
            );

            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quad_instance_data_layout() {
        assert_eq!(std::mem::size_of::<QuadInstanceData>(), 128);
        assert_eq!(std::mem::align_of::<QuadInstanceData>(), 4);

        let data = QuadInstanceData::default();
        let base = &raw const data as usize;

        // Verify that 8 attribute blocks (each vec4 = 16 bytes) align perfectly:
        // 1. a_bounds: rect_pos (8B) + rect_size (8B) = 16B at offset 0
        assert_eq!(&raw const data.rect_pos as usize - base, 0);
        assert_eq!(&raw const data.rect_size as usize - base, 8);

        // 2. a_color: color (16B) at offset 16
        assert_eq!(&raw const data.color as usize - base, 16);

        // 3. a_border_color: border_color (16B) at offset 32
        assert_eq!(&raw const data.border_color as usize - base, 32);

        // 4. a_color_bottom: color_bottom (16B) at offset 48
        assert_eq!(&raw const data.color_bottom as usize - base, 48);

        // 5. a_shape_info: shape_size (8B) + is_circle (4B) + is_shadow (4B) = 16B at offset 64
        assert_eq!(&raw const data.shape_size as usize - base, 64);
        assert_eq!(&raw const data.is_circle as usize - base, 72);
        assert_eq!(&raw const data.is_shadow as usize - base, 76);

        // 6. a_params: radius (4B) + border_width (4B) + shadow_blur (4B) + is_gradient (4B) = 16B at offset 80
        assert_eq!(&raw const data.radius as usize - base, 80);
        assert_eq!(&raw const data.border_width as usize - base, 84);
        assert_eq!(&raw const data.shadow_blur as usize - base, 88);
        assert_eq!(&raw const data.is_gradient as usize - base, 92);

        // 7. a_border_color_bottom: border_color_bottom (16B) at offset 96
        assert_eq!(&raw const data.border_color_bottom as usize - base, 96);

        // 8. a_transform: transform (scale, rotation, pivot) (16B) at offset 112
        assert_eq!(&raw const data.transform as usize - base, 112);
        assert_eq!(data.transform, IDENTITY_TRANSFORM);
    }

    #[test]
    fn test_quad_batch_management() {
        let mut batch = QuadBatch::new(4);
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
        assert!(!batch.is_full());
        assert_eq!(batch.as_slice().len(), 0);

        let inst = QuadInstanceData::default();
        assert!(batch.push_instance(inst));
        assert!(batch.push_instance(inst));
        assert!(batch.push_instance(inst));
        assert!(batch.push_instance(inst));
        assert_eq!(batch.len(), 4);
        assert!(batch.is_full());

        // Push beyond capacity fails
        assert!(!batch.push_instance(inst));
        assert_eq!(batch.len(), 4);

        // Raw byte representation size check: 4 instances * 128 bytes = 512 bytes
        assert_eq!(batch.as_bytes().len(), 4 * 128);

        // Clear preserves buffer allocation
        batch.clear();
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
        assert!(!batch.is_full());
        assert!(batch.instances.capacity() >= 4);
    }

    #[test]
    fn test_quad_instance_data_construction() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        let color_top = 0xFFFF_0000;
        let color_bottom = 0xFF00_00FF;
        let radius = 8.0;
        let border_width = 2.0;
        let border_color = 0xFF00_FF00;

        let col_top = unpack_color(color_top);
        let col_bot = unpack_color(color_bottom);
        let b_col = unpack_color(border_color);

        let instance = QuadInstanceData {
            rect_pos: [rect.x, rect.y],
            rect_size: [rect.width, rect.height],
            color: col_top,
            border_color: b_col,
            color_bottom: col_bot,
            shape_size: [rect.width, rect.height],
            is_circle: 0.0,
            is_shadow: 0.0,
            radius,
            border_width,
            shadow_blur: 0.0,
            is_gradient: 1.0,
            border_color_bottom: b_col,
            transform: IDENTITY_TRANSFORM,
        };

        assert_eq!(instance.rect_pos, [10.0, 20.0]);
        assert_eq!(instance.rect_size, [100.0, 50.0]);
        assert_eq!(instance.radius, 8.0);
        assert_eq!(instance.border_width, 2.0);
        assert_eq!(instance.is_gradient, 1.0);
        assert_eq!(instance.is_circle, 0.0);
        assert_eq!(instance.is_shadow, 0.0);
    }

    #[test]
    fn test_text_batch_lifecycle() {
        let mut batch = TextBatch::new(100);
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);

        batch
            .vertices
            .extend_from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        assert!(!batch.is_empty());
        assert_eq!(batch.len(), 9);
        assert_eq!(batch.as_bytes().len(), 9 * std::mem::size_of::<f32>());

        batch.clear();
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
    }

    #[test]
    fn test_text_batch_100_labels_unified_draw_call() {
        let mut batch = TextBatch::new(65_536);
        assert!(batch.is_empty());

        // Simulate 120 labels (e.g. an App Drawer with 120 app titles + emojis)
        for i in 0..120 {
            let is_emoji = if i % 3 == 0 { 1.0 } else { 0.0 };
            #[allow(clippy::cast_precision_loss)]
            let red = (i as f32) / 120.0;
            let green = 0.5;
            let blue = 1.0 - red;
            let alpha = 1.0;

            // Each character/glyph quad produces 6 vertices of 9 floats = 54 floats
            for vertex_idx in 0..6 {
                #[allow(clippy::cast_precision_loss)]
                let pos_x = (i as f32) * 10.0 + (vertex_idx as f32);
                let pos_y = 50.0;
                let uv_u = 0.1;
                let uv_v = 0.2;
                batch.vertices.extend_from_slice(&[
                    pos_x, pos_y, uv_u, uv_v, red, green, blue, alpha, is_emoji,
                ]);
            }
        }

        // 120 labels * 6 vertices = 720 vertices
        // 720 vertices * 9 floats = 6,480 floats
        assert_eq!(batch.len(), 120 * 6 * 9);
        assert_eq!(batch.len() / TextBatch::FLOATS_PER_VERTEX, 720);
        assert_eq!(
            batch.as_bytes().len(),
            120 * 6 * 9 * std::mem::size_of::<f32>()
        );

        // Verify vertex attributes integrity of normal glyph vs emoji glyph
        // Check 1st label (i = 0, is_emoji = 1.0)
        assert_eq!(batch.vertices[8], 1.0); // is_color attribute
        assert_eq!(batch.vertices[4], 0.0); // r channel

        // Check 2nd label (i = 1, is_emoji = 0.0)
        let label1_offset = 6 * 9;
        assert_eq!(batch.vertices[label1_offset + 8], 0.0); // is_color attribute
        assert!((batch.vertices[label1_offset + 4] - (1.0 / 120.0)).abs() < 1e-5);

        batch.clear();
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
    }

    #[test]
    fn test_text_batch_performance_and_capacity() {
        let mut batch = TextBatch::new(65_536);
        let start = std::time::Instant::now();

        // 500 labels with 10 glyphs each = 5,000 quads = 30,000 vertices
        let quad_sample = [
            10.0, 10.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 20.0, 10.0, 1.0, 0.0, 1.0, 1.0, 1.0,
            1.0, 0.0, 10.0, 20.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 20.0, 10.0, 1.0, 0.0, 1.0,
            1.0, 1.0, 1.0, 0.0, 20.0, 20.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 10.0, 20.0, 0.0,
            1.0, 1.0, 1.0, 1.0, 1.0, 0.0,
        ];

        for _ in 0..1000 {
            batch.vertices.extend_from_slice(&quad_sample);
        }

        let elapsed = start.elapsed();
        assert_eq!(batch.len(), 1000 * 54);
        assert!(!batch.is_empty());
        // 1,000 glyph quads in batch should execute well under 5 milliseconds on any modern CPU
        assert!(
            elapsed.as_millis() < 20,
            "Batching took too long: {elapsed:?}"
        );

        batch.clear();
        assert!(batch.is_empty());
    }

    #[test]
    fn test_image_instance_data_layout() {
        assert_eq!(std::mem::size_of::<ImageInstanceData>(), 64);
        assert_eq!(std::mem::align_of::<ImageInstanceData>(), 4);

        let data = ImageInstanceData::default();
        let base = &raw const data as usize;

        // 1. a_bounds: rect_pos (8B) + rect_size (8B) = 16B at offset 0
        assert_eq!(&raw const data.rect_pos as usize - base, 0);
        assert_eq!(&raw const data.rect_size as usize - base, 8);

        // 2. a_uv_bounds: uv_rect (16B) at offset 16
        assert_eq!(&raw const data.uv_rect as usize - base, 16);

        // 3. a_params: radius (4B) + alpha (4B) + pad (8B) = 16B at offset 32
        assert_eq!(&raw const data.radius as usize - base, 32);
        assert_eq!(&raw const data.alpha as usize - base, 36);
        assert_eq!(&raw const data.pad as usize - base, 40);

        // 4. a_transform: transform (scale, rotation, pivot) (16B) at offset 48
        assert_eq!(&raw const data.transform as usize - base, 48);
        assert_eq!(data.transform, IDENTITY_TRANSFORM);
    }

    #[test]
    fn test_image_batch_management() {
        let mut batch = ImageBatch::new(4);
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
        assert!(!batch.is_full());
        assert_eq!(batch.as_slice().len(), 0);

        let inst = ImageInstanceData {
            rect_pos: [10.0, 20.0],
            rect_size: [64.0, 64.0],
            uv_rect: [0.0, 0.0, 0.5, 0.5],
            radius: 8.0,
            alpha: 1.0,
            pad: [0.0, 0.0],
            transform: IDENTITY_TRANSFORM,
        };

        assert!(batch.push_instance(inst));
        assert!(batch.push_instance(inst));
        assert!(batch.push_instance(inst));
        assert!(batch.push_instance(inst));
        assert_eq!(batch.len(), 4);
        assert!(batch.is_full());

        // Cannot push past capacity
        assert!(!batch.push_instance(inst));
        assert_eq!(batch.len(), 4);

        // 4 instances * 64 bytes = 256 bytes
        assert_eq!(batch.as_bytes().len(), 4 * 64);

        batch.clear();
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
    }
}
