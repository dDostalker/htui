use anyhow::Result;
use env_logger::Builder;
use htui::render;
use log::{debug, error, info, warn};

fn main() -> Result<()> {
    //初始化
    Builder::from_env(env_logger::Env::default().default_filter_or("debug")) //创建日志构建器，读取RUST_LOG,未设置则默认等级debug
        //.target(env_logger::Target::Stderr)控制输出？我还不会用，先留着
        .format_timestamp_secs() //添加时间戳
        .init(); //初始化启动

    info!("程序启动 version:{}", env!("CARGO_PKG_VERSION"));

    let html = r#"<h1>Hello</h1>
    <div><h2>World</h2></div>
    <img src="./test_html/src/pic.png">
    "#;
    let css = "p { color: red; }
        h1 { color: blue; }";

    debug!(
        "准备渲染 HTML 长度:{} 字节，CSS 长度:{} 字节",
        html.len(),
        css.len()
    );
    render(html, css);
    info!("渲染完成");
    Ok(())
}
