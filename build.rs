fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("app.ico");
        res.set("ProductName", "UART Assistant");
        res.set("FileDescription", "UART Assistant serial debug assistant");
        res.set("ProductVersion", "0.2.0");
        res.set("FileVersion", "0.2.0");
        res.set("LegalCopyright", "MIT License");
        if let Err(err) = res.compile() {
            eprintln!("warning: failed to embed windows resources: {err}");
        }
    }
}
