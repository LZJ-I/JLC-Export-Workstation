//! 用 eframe 同一套 glow 上下文画 3D，避免再开窗口。

use eframe::egui::{self, PaintCallbackInfo};
use eframe::egui_glow::glow::{self, HasContext};
use eframe::egui_glow::{self, ShaderVersion};
use lceda_core::mesh::Mesh;
use std::sync::Arc;

pub struct GpuPreview {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    vert_count: i32,
    uploaded: usize,
}

impl GpuPreview {
    pub fn new(gl: &glow::Context) -> Option<Self> {
        let version = ShaderVersion::get(gl);
        if !version.is_new_shader_interface() {
            return None;
        }
        let prefix = version.version_declaration();
        unsafe {
            let program = compile_program(gl, &prefix)?;
            let vao = gl.create_vertex_array().ok()?;
            let vbo = gl.create_buffer().ok()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = (9 * 4) as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, 12);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 3, glow::FLOAT, false, stride, 24);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            Some(Self {
                program,
                vao,
                vbo,
                vert_count: 0,
                uploaded: 0,
            })
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_buffer(self.vbo);
            gl.delete_vertex_array(self.vao);
        }
    }

    pub fn paint(
        &mut self,
        gl: &glow::Context,
        info: &PaintCallbackInfo,
        mesh: &Mesh,
        yaw: f32,
        pitch: f32,
        zoom: f32,
        bg: [f32; 4],
    ) {
        let vp = info.viewport_in_pixels();
        if vp.width_px <= 0 || vp.height_px <= 0 {
            return;
        }
        self.sync_mesh(gl, mesh);
        if self.vert_count <= 0 {
            return;
        }
        let w = vp.width_px as f32;
        let h = vp.height_px as f32;
        let min_side = w.min(h);
        let sx = 1.44 * zoom * min_side / w;
        let sy = 1.44 * zoom * min_side / h;
        unsafe {
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.enable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.viewport(vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px);
            gl.scissor(vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px);
            gl.clear_color(bg[0], bg[1], bg[2], bg[3]);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.use_program(Some(self.program));
            gl.uniform_1_f32(gl.get_uniform_location(self.program, "u_yaw").as_ref(), yaw);
            gl.uniform_1_f32(
                gl.get_uniform_location(self.program, "u_pitch").as_ref(),
                pitch,
            );
            gl.uniform_1_f32(gl.get_uniform_location(self.program, "u_sx").as_ref(), sx);
            gl.uniform_1_f32(gl.get_uniform_location(self.program, "u_sy").as_ref(), sy);
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, self.vert_count);
            gl.bind_vertex_array(None);
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
        }
    }

    fn sync_mesh(&mut self, gl: &glow::Context, mesh: &Mesh) {
        let id = mesh as *const Mesh as usize;
        if id == self.uploaded && self.vert_count > 0 {
            return;
        }
        let data = pack_mesh(mesh);
        self.vert_count = (data.len() / 36) as i32;
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &data, glow::STATIC_DRAW);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        self.uploaded = id;
    }
}

fn pack_mesh(mesh: &Mesh) -> Vec<u8> {
    let (min, max) = mesh.vertices.iter().fold(
        ([f32::MAX; 3], [f32::MIN; 3]),
        |(mut min, mut max), v| {
            for i in 0..3 {
                min[i] = min[i].min(v[i]);
                max[i] = max[i].max(v[i]);
            }
            (min, max)
        },
    );
    let cx = (min[0] + max[0]) * 0.5;
    let cy = (min[1] + max[1]) * 0.5;
    let cz = (min[2] + max[2]) * 0.5;
    let span = (max[0] - min[0])
        .max(max[1] - min[1])
        .max(max[2] - min[2])
        .max(1e-3);
    let mut data = Vec::with_capacity(mesh.triangles.len() * 3 * 36);
    for (i, tri) in mesh.triangles.iter().enumerate() {
        let a = mesh.vertices.get(tri[0] as usize).copied().unwrap_or([0.0; 3]);
        let b = mesh.vertices.get(tri[1] as usize).copied().unwrap_or([0.0; 3]);
        let c = mesh.vertices.get(tri[2] as usize).copied().unwrap_or([0.0; 3]);
        let pa = [
            (a[0] - cx) / span,
            (a[1] - cy) / span,
            (a[2] - cz) / span,
        ];
        let pb = [
            (b[0] - cx) / span,
            (b[1] - cy) / span,
            (b[2] - cz) / span,
        ];
        let pc = [
            (c[0] - cx) / span,
            (c[1] - cy) / span,
            (c[2] - cz) / span,
        ];
        let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let mut n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        n[0] /= nl;
        n[1] /= nl;
        n[2] /= nl;
        let rgb = mesh.tri_rgb.get(i).copied().unwrap_or([196, 196, 200]);
        let col = [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0];
        for p in [pa, pb, pc] {
            push3(&mut data, p);
            push3(&mut data, col);
            push3(&mut data, n);
        }
    }
    data
}

fn push3(buf: &mut Vec<u8>, v: [f32; 3]) {
    for x in v {
        buf.extend_from_slice(&x.to_le_bytes());
    }
}

unsafe fn compile_program(gl: &glow::Context, prefix: &str) -> Option<glow::Program> {
    let vs = r#"
        layout(location = 0) in vec3 a_pos;
        layout(location = 1) in vec3 a_color;
        layout(location = 2) in vec3 a_normal;
        uniform float u_yaw;
        uniform float u_pitch;
        uniform float u_sx;
        uniform float u_sy;
        out vec3 v_color;
        void main() {
            float cy = cos(u_yaw);
            float sy = sin(u_yaw);
            float cp = cos(u_pitch);
            float sp = sin(u_pitch);
            float x1 = a_pos.x * cy - a_pos.y * sy;
            float y1 = a_pos.x * sy + a_pos.y * cy;
            float y2 = y1 * cp - a_pos.z * sp;
            float z2 = y1 * sp + a_pos.z * cp;
            gl_Position = vec4(x1 * u_sx, y2 * u_sy, z2 * 0.45, 1.0);
            float nx = a_normal.x * cy - a_normal.y * sy;
            float ny1 = a_normal.x * sy + a_normal.y * cy;
            float ny = ny1 * cp - a_normal.z * sp;
            float nz = ny1 * sp + a_normal.z * cp;
            float shade = 0.30 + 0.70 * max(0.0, nx * 0.25 + ny * 0.70 + nz * 0.55);
            v_color = a_color * shade;
        }
    "#;
    let fs = r#"
        precision mediump float;
        in vec3 v_color;
        out vec4 out_color;
        void main() {
            out_color = vec4(v_color, 1.0);
        }
    "#;
    let program = gl.create_program().ok()?;
    let shaders = [
        (glow::VERTEX_SHADER, vs),
        (glow::FRAGMENT_SHADER, fs),
    ];
    let mut compiled = Vec::new();
    for (kind, src) in shaders {
        let shader = gl.create_shader(kind).ok()?;
        gl.shader_source(shader, &format!("{prefix}\n{src}"));
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            gl.delete_shader(shader);
            gl.delete_program(program);
            return None;
        }
        gl.attach_shader(program, shader);
        compiled.push(shader);
    }
    gl.link_program(program);
    for shader in compiled {
        gl.detach_shader(program, shader);
        gl.delete_shader(shader);
    }
    if !gl.get_program_link_status(program) {
        gl.delete_program(program);
        return None;
    }
    Some(program)
}

pub fn paint_callback(
    rect: egui::Rect,
    gpu: Arc<egui::mutex::Mutex<GpuPreview>>,
    mesh: Arc<Mesh>,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    bg: egui::Color32,
) -> egui::PaintCallback {
    let bg = [
        bg.r() as f32 / 255.0,
        bg.g() as f32 / 255.0,
        bg.b() as f32 / 255.0,
        bg.a() as f32 / 255.0,
    ];
    let cb = egui_glow::CallbackFn::new(move |info, painter| {
        gpu.lock()
            .paint(painter.gl(), &info, mesh.as_ref(), yaw, pitch, zoom, bg);
    });
    egui::PaintCallback {
        rect,
        callback: std::sync::Arc::new(cb),
    }
}
