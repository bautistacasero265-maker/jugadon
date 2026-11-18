// Mete el ícono y los datos de versión (nombre que muestra el Administrador de tareas) en el exe.
fn main() {
    println!("cargo:rerun-if-changed=assets/jugadon.rc");
    println!("cargo:rerun-if-changed=assets/jugadon.ico");
    println!("cargo:rerun-if-changed=assets/jugadon_pausa.ico");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        embed_resource::compile("assets/jugadon.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
