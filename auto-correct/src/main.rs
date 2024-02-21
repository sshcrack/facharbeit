use std::{
    env::{args, current_exe}, thread, time::Duration
};

use anyhow::Result;
use clipboard_win::set_clipboard_string;
use punkt::{params::Standard, SentenceTokenizer, Trainer, TrainingData};
use thirtyfour::{
    components::Component,
    extensions::query::{ElementQueryable, ElementWaitable},
    By, DesiredCapabilities, Key, WebDriver, WebElement,
};
use tokio::{
    fs::{self, File},
    io::AsyncReadExt,
    process::Command,
};

#[tokio::main]
async fn main() -> Result<()> {
    let file = args()
        .skip(1)
        .next()
        .expect("Argument to filename not given");
    println!("Opening {}", file);

    let mut f = File::open(file).await?;
    let mut latex_doc = String::new();

    f.read_to_string(&mut latex_doc).await?;
    drop(f);

    #[cfg(feature = "chrome")]
    let bin = include_bytes!("./bin/chromedriver.exe");

    #[cfg(not(feature = "chrome"))]
    let bin = include_bytes!("./bin/geckodriver.exe");

    let driver_file = current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .with_file_name("driver.exe");
    println!("Writing Driver at {:?}", driver_file);
    fs::write(&driver_file, bin).await?;

    println!("Driver wird gestartet...");
    let mut child = Command::new(driver_file).spawn()?;

    let driver = prepare_deepl().await?;
    let out_str = correct_deepl(&driver, &latex_doc).await?;

    fs::write("corrected.tex", out_str).await?;
    println!("Driver wird beendet...");
    driver.close_window().await?;
    child.start_kill()?;
    child.wait().await?;

    Ok(())
}

pub async fn prepare_deepl() -> Result<WebDriver> {
    println!("Browser wird geöffnet...");

    #[cfg(feature = "chrome")]
    #[allow(unused_variables)]
    let caps = DesiredCapabilities::chrome();
    #[allow(unused_variables)]
    #[cfg(feature = "firefox")]
    let caps = DesiredCapabilities::firefox();

    #[cfg(feature = "chrome")]
    #[allow(unused_variables)]
    let url = "http://localhost:9515";
    #[cfg(feature = "firefox")]
    #[allow(unused_variables)]
    let url = "http://localhost:4444";

    let driver = WebDriver::new(url, caps).await?;
    driver.goto("https://www.deepl.com/write").await?;
    driver.set_script_timeout(Duration::new(60 * 60 * 24, 0)).await?;

    //find_and_press_btn(
    //        &driver,
    //        By::XPath("//*[@id=\"write-text-styles-popover-button\"]"),
    //    )
    //    .await?;
    /*find_and_press_btn(
        &driver,
        By::Css("button.border-neutral-next-100:nth-child(3)"),
    )
    .await?;
    find_and_press_btn(&driver, By::Css("button.inline-flex:nth-child(2)")).await?;*/
    find_and_press_btn(&driver, By::Css("#headlessui-listbox-button-28")).await?;
    find_and_press_btn(&driver, By::Css("#headlessui-listbox-option-32")).await?;

    Ok(driver)
}

pub async fn correct_deepl(driver: &WebDriver, text: &str) -> Result<String> {
    let mut text_done = String::new();
    let mut to_process = String::new();
    let mut to_prepend = String::new();

    let mut has_started = false;
    let mut has_ended = false;
    for line in text.lines() {
        if !has_started && !has_ended {
            text_done = text_done + line + "\n";
        }

        if has_started && !has_ended {
            to_process = to_process + line + "\n"
        }

        if has_started && has_ended {
            to_prepend = to_prepend + line + "\n";
        }

        if line == "%CORRECT_START" {
            has_started = true;
        }

        if line == "%CORRECT_END" {
            has_ended = true;
        }
    }

    let mut curr_chunk = String::new();
    let mut curr_environment = Vec::new();
    for line in to_process.lines() {
        let is_begin = line.starts_with("\\begin{");
        let is_end = line.starts_with("\\end{");
        if is_begin || is_end {
            let env_name = line.split("{").last().unwrap();
            let env_name = env_name.split("}").next().unwrap();

            if is_begin {
                curr_environment.push(env_name);
            }

            if is_end {
                let to_remove = curr_environment.iter().position(|e| *e == env_name);
                if let Some(e) = to_remove {
                    curr_environment.remove(e);
                }
            }
        }

        if line.starts_with("%") || line.starts_with("\\") || curr_environment.len() != 0 {
            text_done = text_done + line + "\n";
            if !curr_chunk.is_empty() {
                let trainer: Trainer<Standard> = Trainer::new();
                let mut data = TrainingData::german();

                println!("Training");
                trainer.train(&curr_chunk, &mut data);
                let sentences: Vec<&str> =
                    SentenceTokenizer::<Standard>::new(&curr_chunk, &data).collect();
                let improved = improve_deepl(driver, &sentences).await?;

                println!("Writing {}", improved);
                text_done.push_str(&improved);
                text_done.push_str("\n");
                curr_chunk = String::new();
            }
            continue;
        }

        curr_chunk = curr_chunk + line + "\n";
    }

    text_done.push_str(&to_prepend);
    Ok(text_done)
}

async fn improve_deepl(driver: &WebDriver, sentences: &Vec<&str>) -> Result<String> {
    let mut curr_merged = String::new();
    let mut corrected = Vec::new();
    for sentence in sentences {
        if sentence.len() + curr_merged.len() > 2000 {
            let res = improve_deepl_raw(driver, &curr_merged).await?;
            corrected.push(res);

            curr_merged = sentence.to_string();
            continue;
        }

        if !curr_merged.is_empty() {
            curr_merged.push(' ');
        }
    
        curr_merged.push_str(&sentence);
    }

    if !curr_merged.is_empty() {
        let res = improve_deepl_raw(driver, &curr_merged).await?;
        corrected.push(res);
    }

    Ok(corrected.join(" "))
}

async fn improve_deepl_raw(driver: &WebDriver, raw: &str) -> Result<String> {
    let input = driver
        .query(By::Css(".min-h-0 > div:nth-child(1)"))
        .first()
        .await?;
    let output = driver
        .query(By::Css(".last\\:grow > div:nth-child(1)"))
        .first()
        .await?;

    let mut prev = output.text().await?;

    loop {
        if raw.trim().is_empty() {
            return Ok("".to_string());
        }

        set_clipboard_string(&raw).expect("To set clipboard");

        input.send_keys(Key::Control + "a".to_string()).await?;
        thread::sleep(Duration::from_millis(100));
        input.send_keys(Key::Control + "v".to_string()).await?;

        for i in 0..30 {
            println!("Waiting for change ({}/30)...", i);
            thread::sleep(Duration::from_millis(500));

            let temp = output.text().await?;
            if &temp != &prev {
                println!("Waiting for approval...");
                driver.execute_async(r#"
                let done = arguments[0];
                const func = e => {
                    console.log("Key", e)
                    if(e.key !== "b" || !e.ctrlKey)
                        return
                    window.removeEventListener("keydown", func)
                    done()
                }
                window.addEventListener("keydown", func)
                "#, Vec::new()).await?;

                return Ok(output.text().await?);
            }

            prev = temp;
        }

        println!("Trying again...");
    }
}

async fn find_and_press_btn(driver: &WebDriver, q: By) -> Result<()> {
    let apply = driver.query(q).first().await?;

    apply.wait_until();
    apply.click().await?;

    thread::sleep(Duration::from_millis(500));
    Ok(())
}

/// This component shows how to nest components inside others.
#[derive(Debug, Clone, Component)]
pub struct WrapperComponent {
    base: WebElement, // This is the outer <div>
}
