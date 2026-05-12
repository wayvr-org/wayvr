pub mod vert_quad {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "src/shaders/quad.vert"
    }
}

pub mod frag_color {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/shaders/color.frag",
    }
}

pub mod frag_grid {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/shaders/grid.frag",
    }
}

pub mod frag_screen {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/shaders/screen.frag",
    }
}

pub mod frag_srgb {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/shaders/srgb.frag",
    }
}

pub mod frag_sky {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "src/shaders/sky.frag",
    }
}
