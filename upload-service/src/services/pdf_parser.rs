use std::fs::File;
use std::io::Read;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

/// Rust原生PDF解析器
pub struct PdfParser;

impl PdfParser {
    /// 解析PDF文件
    pub async fn parse_pdf(file_path: &Path) -> Result<ParsedDocument, anyhow::Error> {
        tracing::info!("📄 开始解析PDF: {:?}", file_path);

        let start = std::time::Instant::now();

        // 直接使用lopdf（更稳定，支持中文PDF）
        let text = Self::parse_pdf_with_lopdf(file_path)?;

        let duration = start.elapsed();
        tracing::info!(
            "✅ PDF解析完成，耗时: {:?}, 文本长度: {}",
            duration,
            text.len()
        );

        let page_count = Self::estimate_page_count(&text);

        Ok(ParsedDocument {
            text,
            page_count,
            file_type: "pdf".to_string(),
            parse_duration_ms: duration.as_millis() as i64,
        })
    }

    /// 备用方案：使用lopdf解析（支持更多PDF格式）
    fn parse_pdf_with_lopdf(file_path: &Path) -> Result<String, anyhow::Error> {
        tracing::info!("📄 使用lopdf备用方案解析");

        use lopdf::Document;

        let doc = Document::load(file_path).map_err(|e| anyhow::anyhow!("lopdf加载失败: {}", e))?;

        let mut text = String::new();
        let pages = doc.get_pages();

        for (page_num, _) in pages.iter() {
            if let Ok(page_text) = doc.extract_text(&[*page_num]) {
                text.push_str(&page_text);
                text.push('\n');
            }
        }

        if text.is_empty() {
            // 如果lopdf也无法提取文本，返回基本信息
            text = format!(
                "【PDF文档】\n文件名: {:?}\n页数: {}\n（文本提取失败，可能是扫描件或加密PDF）",
                file_path.file_name().unwrap_or_default(),
                pages.len()
            );
        }

        tracing::info!("✅ lopdf解析完成，提取 {} 字符", text.len());
        Ok(text)
    }

    /// 解析DOCX文件
    pub async fn parse_docx(file_path: &Path) -> Result<ParsedDocument, anyhow::Error> {
        tracing::info!("📄 开始解析DOCX: {:?}", file_path);

        let start = std::time::Instant::now();

        // 读取DOCX文件内容
        let file_data = tokio::fs::read(file_path).await?;
        let doc =
            docx_rs::read_docx(&file_data).map_err(|e| anyhow::anyhow!("DOCX解析失败: {:?}", e))?;

        // 提取文本
        let mut text = Self::extract_text_from_docx(&doc);

        // 如果提取结果为空，尝试使用XML解析作为兜底（兼容textutil转换的DOCX）
        if text.trim().is_empty() {
            tracing::warn!(
                "⚠️ DOCX主解析器未提取到文本，尝试XML兜底解析: {:?}",
                file_path
            );
            match Self::extract_text_from_docx_zip(file_path) {
                Ok(fallback) if !fallback.trim().is_empty() => {
                    tracing::info!("✅ DOCX兜底解析成功，提取 {} 字符", fallback.len());
                    text = fallback;
                }
                Ok(_) => {
                    tracing::error!("❌ DOCX兜底解析仍为空，请检查文档内容");
                }
                Err(err) => {
                    tracing::error!("❌ DOCX兜底解析失败: {}", err);
                }
            }
        }

        let duration = start.elapsed();
        tracing::info!(
            "✅ DOCX解析完成，耗时: {:?}, 文本长度: {}",
            duration,
            text.len()
        );

        let page_count = Self::estimate_page_count(&text);

        Ok(ParsedDocument {
            text,
            page_count,
            file_type: "docx".to_string(),
            parse_duration_ms: duration.as_millis() as i64,
        })
    }

    /// 统一解析入口
    pub async fn parse(file_path: &Path) -> Result<ParsedDocument, anyhow::Error> {
        let extension = file_path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| anyhow::anyhow!("无效的文件类型"))?
            .to_lowercase();

        match extension.as_str() {
            "pdf" => Self::parse_pdf(file_path).await,
            "docx" => Self::parse_docx(file_path).await,
            "xlsx" | "xls" => Self::parse_excel(file_path).await,
            "html" | "htm" => Self::parse_html(file_path).await,
            "md" | "markdown" => Self::parse_markdown(file_path).await,
            "txt" => Self::parse_text(file_path).await,
            "doc" => {
                // DOC格式较老，可以提示用户转换为DOCX
                Err(anyhow::anyhow!("不支持.doc格式，请转换为.docx或.pdf"))
            }
            _ => Err(anyhow::anyhow!("不支持的文件格式: {}", extension)),
        }
    }

    /// 解析Excel文件 (.xlsx, .xls)
    pub async fn parse_excel(file_path: &Path) -> Result<ParsedDocument, anyhow::Error> {
        tracing::info!("📊 开始解析Excel: {:?}", file_path);

        let start = std::time::Instant::now();

        use calamine::{open_workbook_auto, Reader};

        // 打开工作簿
        let mut workbook =
            open_workbook_auto(file_path).map_err(|e| anyhow::anyhow!("Excel打开失败: {:?}", e))?;

        let mut text = String::new();
        let sheet_names = workbook.sheet_names().to_vec();
        let sheet_count = sheet_names.len();

        tracing::info!("📋 Excel包含 {} 个工作表", sheet_count);

        // 遍历所有工作表
        for sheet_name in sheet_names {
            // ✅【优化】工作表名作为段落标题
            text.push_str(&format!("\n━━━━━━ {} ━━━━━━\n\n", sheet_name));

            if let Ok(range) = workbook.worksheet_range(&sheet_name) {
                let rows: Vec<_> = range.rows().collect();

                // ✅【优化】提取表头（第一行作为标题）
                if let Some(header_row) = rows.first() {
                    let headers: Vec<String> = header_row
                        .iter()
                        .map(|cell| format!("{}", cell))
                        .filter(|s| !s.is_empty() && s != "Empty")
                        .collect();

                    if !headers.is_empty() {
                        text.push_str("【表头】");
                        text.push_str(&headers.join(" | "));
                        text.push_str("\n\n");

                        // ✅【优化】关键表头关键词高亮（用于JD识别）
                        let header_text = headers.join(" ").to_lowercase();
                        if header_text.contains("岗位")
                            || header_text.contains("职位")
                            || header_text.contains("需求")
                            || header_text.contains("招聘")
                        {
                            text.push_str("【关键标识】岗位招聘表格\n\n");
                        }
                    }
                }

                // ✅ 遍历数据行（跳过表头）
                for (idx, row) in rows.iter().enumerate().skip(1) {
                    let row_text: Vec<String> = row
                        .iter()
                        .map(|cell| format!("{}", cell))
                        .filter(|s| !s.is_empty() && s != "Empty")
                        .collect();

                    if !row_text.is_empty() {
                        // ✅【优化】保留行结构，用管道分隔
                        text.push_str(&format!("第{}行: ", idx + 1));
                        text.push_str(&row_text.join(" | "));
                        text.push('\n');
                    }
                }
            }

            text.push_str("\n");
        }

        let duration = start.elapsed();
        tracing::info!(
            "✅ Excel解析完成，耗时: {:?}, 文本长度: {}",
            duration,
            text.len()
        );

        let page_count = sheet_count as u32;

        Ok(ParsedDocument {
            text,
            page_count,
            file_type: "excel".to_string(),
            parse_duration_ms: duration.as_millis() as i64,
        })
    }

    /// 解析HTML文件
    pub async fn parse_html(file_path: &Path) -> Result<ParsedDocument, anyhow::Error> {
        tracing::info!("🌐 开始解析HTML: {:?}", file_path);

        let start = std::time::Instant::now();

        // 读取HTML文件
        let html_content = tokio::fs::read_to_string(file_path).await?;

        use scraper::{Html, Selector};

        // 解析HTML
        let document = Html::parse_document(&html_content);

        let mut text = String::new();

        // 提取标题
        if let Ok(title_selector) = Selector::parse("title") {
            for element in document.select(&title_selector) {
                text.push_str(&format!(
                    "【标题】{}\n\n",
                    element.text().collect::<String>()
                ));
            }
        }

        // 提取主要文本内容（去除script和style标签）
        if let Ok(body_selector) = Selector::parse("body") {
            for element in document.select(&body_selector) {
                let body_text = element
                    .text()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                text.push_str(&body_text);
            }
        }

        // 如果没有body标签，提取所有文本
        if text.is_empty() {
            text = document
                .root_element()
                .text()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
        }

        let duration = start.elapsed();
        tracing::info!(
            "✅ HTML解析完成，耗时: {:?}, 文本长度: {}",
            duration,
            text.len()
        );

        let page_count = 1; // HTML视为单页

        Ok(ParsedDocument {
            text,
            page_count,
            file_type: "html".to_string(),
            parse_duration_ms: duration.as_millis() as i64,
        })
    }

    /// 解析Markdown文件
    pub async fn parse_markdown(file_path: &Path) -> Result<ParsedDocument, anyhow::Error> {
        tracing::info!("📝 开始解析Markdown: {:?}", file_path);

        let start = std::time::Instant::now();

        // 读取Markdown文件
        let content = tokio::fs::read_to_string(file_path).await?;

        // Markdown是纯文本，但可以选择清理一些标记符号以提高AI解析质量
        let text = Self::clean_markdown(&content);

        let duration = start.elapsed();
        tracing::info!(
            "✅ Markdown解析完成，耗时: {:?}, 文本长度: {}",
            duration,
            text.len()
        );

        let page_count = Self::estimate_page_count(&text);

        Ok(ParsedDocument {
            text,
            page_count,
            file_type: "markdown".to_string(),
            parse_duration_ms: duration.as_millis() as i64,
        })
    }

    /// 解析纯文本文件
    pub async fn parse_text(file_path: &Path) -> Result<ParsedDocument, anyhow::Error> {
        tracing::info!("📄 开始解析文本文件: {:?}", file_path);

        let start = std::time::Instant::now();

        // 读取文本文件
        let text = tokio::fs::read_to_string(file_path).await?;

        let duration = start.elapsed();
        tracing::info!(
            "✅ 文本解析完成，耗时: {:?}, 文本长度: {}",
            duration,
            text.len()
        );

        let page_count = Self::estimate_page_count(&text);

        Ok(ParsedDocument {
            text,
            page_count,
            file_type: "text".to_string(),
            parse_duration_ms: duration.as_millis() as i64,
        })
    }

    /// 清理Markdown标记，保留纯文本（可选）
    fn clean_markdown(content: &str) -> String {
        let mut text = content.to_string();

        // 保留大部分Markdown内容，只做轻量清理
        // 移除代码块标记（```）但保留代码内容
        text = text.replace("```rust", "");
        text = text.replace("```python", "");
        text = text.replace("```javascript", "");
        text = text.replace("```java", "");
        text = text.replace("```go", "");
        text = text.replace("```", "");

        // 清理标题符号（保留文本）
        for i in (1..=6).rev() {
            let marker = "#".repeat(i);
            text = text.replace(&format!("{} ", marker), "");
        }

        // 清理粗体和斜体标记
        text = text.replace("**", "");
        text = text.replace("__", "");
        text = text.replace("*", "");
        text = text.replace("_", "");

        // 清理链接标记，保留文本
        // [文本](链接) -> 文本
        let link_re = regex::Regex::new(r"\[([^\]]+)\]\([^\)]+\)").unwrap();
        text = link_re.replace_all(&text, "$1").to_string();

        // 清理图片标记
        let img_re = regex::Regex::new(r"!\[([^\]]*)\]\([^\)]+\)").unwrap();
        text = img_re.replace_all(&text, "").to_string();

        // 多余空行压缩
        let multi_newline_re = regex::Regex::new(r"\n{3,}").unwrap();
        text = multi_newline_re.replace_all(&text, "\n\n").to_string();

        text.trim().to_string()
    }

    /// 从DOCX提取文本
    fn extract_text_from_docx(doc: &docx_rs::Docx) -> String {
        let mut text = String::new();

        // 遍历文档内容
        for child in &doc.document.children {
            match child {
                docx_rs::DocumentChild::Paragraph(para) => {
                    for child in &para.children {
                        if let docx_rs::ParagraphChild::Run(run) = child {
                            for child in &run.children {
                                if let docx_rs::RunChild::Text(t) = child {
                                    text.push_str(&t.text);
                                }
                            }
                        }
                    }
                    text.push('\n');
                }
                _ => {}
            }
        }

        text
    }

    /// 使用ZIP + XML解析从DOCX提取文本（兼容更多生成方式）
    fn extract_text_from_docx_zip(file_path: &Path) -> Result<String, anyhow::Error> {
        let file = File::open(file_path).map_err(|e| anyhow::anyhow!("打开DOCX文件失败: {}", e))?;
        let mut archive =
            ZipArchive::new(file).map_err(|e| anyhow::anyhow!("解析DOCX压缩包失败: {}", e))?;

        let mut document_xml = archive
            .by_name("word/document.xml")
            .map_err(|e| anyhow::anyhow!("读取word/document.xml失败: {}", e))?;

        let mut xml_content = String::new();
        document_xml
            .read_to_string(&mut xml_content)
            .map_err(|e| anyhow::anyhow!("读取DOCX XML内容失败: {}", e))?;

        let mut reader = Reader::from_str(&xml_content);
        reader.trim_text(true);
        let mut buf = Vec::new();
        let mut text = String::new();
        let mut needs_newline = false;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    if e.name().as_ref() == b"w:p" {
                        needs_newline = true;
                    }
                }
                Ok(Event::Text(e)) => {
                    if needs_newline && !text.ends_with('\n') {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        needs_newline = false;
                    }
                    match e.unescape() {
                        Ok(content) => text.push_str(&content),
                        Err(err) => {
                            tracing::warn!("⚠️ DOCX文本转义失败: {}", err);
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"w:p" {
                        needs_newline = false;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => {
                    return Err(anyhow::anyhow!("解析DOCX XML失败: {}", err));
                }
                _ => {}
            }
            buf.clear();
        }

        Ok(text.trim().to_string())
    }

    /// 估算页数（基于文本长度）
    fn estimate_page_count(text: &str) -> u32 {
        // 假设每页约2000个字符
        let chars = text.len();
        let pages = (chars / 2000).max(1);
        pages as u32
    }
}

#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub text: String,
    pub page_count: u32,
    #[allow(dead_code)]
    pub file_type: String,
    #[allow(dead_code)]
    pub parse_duration_ms: i64,
}
