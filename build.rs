// Bettet unter Windows das Programmsymbol in die .exe ein (Explorer, Taskleiste, Verknüpfungen).
fn main() {
    #[cfg(windows)]
    {
        winresource::WindowsResource::new().set_icon("packaging/windows/icon.ico").compile().expect("Icon einbetten fehlgeschlagen");
    }
}
