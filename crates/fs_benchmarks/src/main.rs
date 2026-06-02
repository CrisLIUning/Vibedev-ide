use fs::Fs;
use gpui::AppContext;
use gpui_platform::headless;

fn main() {
    let Some(path_to_read) = std::env::args().nth(1) else {
        println!("预期第 1 个参数为要读取的路径。");
        return;
    };

    let _ = headless().run(|cx| {
        let fs = fs::RealFs::new(None, cx.background_executor().clone());
        cx.background_spawn(async move {
            let timer = std::time::Instant::now();
            let result = fs.load_bytes(path_to_read.as_ref()).await;
            let elapsed = timer.elapsed();
            if let Err(e) = result {
                println!("在 {elapsed:?} 后 `load_bytes` 失败,错误为 `{e}`");
            } else {
                println!("读取 {} 字节耗时 {elapsed:?}", result.unwrap().len());
            };
            let timer = std::time::Instant::now();
            let result = fs.metadata(path_to_read.as_ref()).await;
            let elapsed = timer.elapsed();
            if let Err(e) = result {
                println!("在 {elapsed:?} 后 `metadata` 失败,错误为 `{e}`");
            } else {
                println!("查询元数据耗时 {elapsed:?}");
            };
            std::process::exit(0);
        })
        .detach();
    });
}
