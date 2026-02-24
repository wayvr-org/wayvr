use std::io::Result;
fn main() -> Result<()> {
    prost_build::compile_protos(&["src/monado_metrics.proto"], &["src/"])?;
    Ok(())
}
