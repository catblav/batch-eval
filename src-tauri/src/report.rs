//! 完整评测报告（xlsx）。
//!
//! 前端把「分析页上能看到的一切」序列化成 payload 传进来，这里只负责排版：
//! 概览 / 错误分类 / 明细，三个 sheet 一次成型。
//!
//! 为什么放后端：多 sheet、单元格底色、冻结表头、百分比格式都要真 xlsx，
//! rust_xlsxwriter 是现成的，前端没有同等可靠的写样式库。

use rust_xlsxwriter::{Color, DocProperties, Format, FormatAlign, FormatBorder, Workbook, Worksheet};
use serde::{Deserialize, Serialize};

/// Excel 单元格上限 32767 字符，留点余量给截断后缀
const CELL_LIMIT: usize = 30000;

// ---------- 与前端约定的数据结构 ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportPayload {
    /// 本期评测名称，作为报告文档标题
    #[serde(default)]
    pub title: String,
    /// 方案名与数据集名，概览里展示
    #[serde(default)]
    pub plan_name: String,
    #[serde(default)]
    pub dataset_name: String,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub dataset_rows: usize,
    /// 被测模型与评测模型的调用参数，报告里并排展示
    #[serde(default)]
    pub target_profile: ProfileInfo,
    #[serde(default)]
    pub judge_profile: ProfileInfo,
    pub summary: Summary,
    #[serde(default)]
    pub categories: Vec<CatItem>,
    #[serde(default)]
    pub rows: Vec<ReportRow>,
}

/// 一张 API 配置里需要进报告的部分。刻意不含 apiKey / baseUrl。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub context_window: f64,
    #[serde(default)]
    pub max_tokens: f64,
    #[serde(default)]
    pub temperature: f64,
    #[serde(default)]
    pub top_p: f64,
    #[serde(default)]
    pub timeout_secs: f64,
    #[serde(default)]
    pub extra_body: String,
    #[serde(default)]
    pub extra_headers: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub total: usize,
    pub valid: usize,
    pub passed: usize,
    pub failed: usize,
    pub excluded: usize,
    pub pass_rate: f64,
    pub avg_target_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CatItem {
    #[serde(default)]
    pub name: String,
    pub count: usize,
    /// 失败分类里是「占不通过的比例」，未计入里不使用
    #[serde(default)]
    pub ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportRow {
    pub index: usize,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub excluded: bool,
    #[serde(default)]
    pub categories: String,
    #[serde(default)]
    pub reasons: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub target_error: String,
    #[serde(default)]
    pub target_latency_ms: u64,
    #[serde(default)]
    pub judge_latency_ms: u64,
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub judge_raw: String,
}

// ---------- 配色（与界面同一套语义） ----------

const BRAND: Color = Color::RGB(0x3D6FA8);
const BRAND_SOFT: Color = Color::RGB(0xEDF2F9);
const PASS_BG: Color = Color::RGB(0xE6F4EC);
const PASS_FG: Color = Color::RGB(0x1E8E5A);
const FAIL_BG: Color = Color::RGB(0xFBEAEA);
const FAIL_FG: Color = Color::RGB(0xC4473F);
const WARN_BG: Color = Color::RGB(0xFCF3E2);
const WARN_FG: Color = Color::RGB(0xB07A22);
const TEXT: Color = Color::RGB(0x2B2B2B);
const MUTED: Color = Color::RGB(0x6B7280);
const BORDER: Color = Color::RGB(0xD8DDE3);
const ZEBRA: Color = Color::RGB(0xF7F8FA);

// ---------- 格式 ----------

fn title_fmt() -> Format {
    Format::new()
        .set_bold()
        .set_font_size(18)
        .set_font_color(Color::White)
        .set_background_color(BRAND)
        .set_align(FormatAlign::VerticalCenter)
        .set_align(FormatAlign::Left)
        .set_indent(1)
}

fn section_fmt() -> Format {
    Format::new()
        .set_bold()
        .set_font_size(11)
        .set_font_color(BRAND)
        .set_border_bottom(FormatBorder::Medium)
        .set_border_bottom_color(BRAND)
        .set_align(FormatAlign::Bottom)
}

fn head_fmt() -> Format {
    Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(BRAND)
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter)
        .set_border(FormatBorder::Thin)
        .set_border_color(BORDER)
}

fn cell_fmt() -> Format {
    Format::new()
        .set_font_color(TEXT)
        .set_border(FormatBorder::Thin)
        .set_border_color(BORDER)
        .set_align(FormatAlign::Top)
}

fn wrap_fmt() -> Format {
    cell_fmt().set_text_wrap()
}

fn num_fmt() -> Format {
    cell_fmt().set_align(FormatAlign::Center)
}

fn pct_fmt() -> Format {
    Format::new()
        .set_num_format("0.0%")
        .set_border(FormatBorder::Thin)
        .set_border_color(BORDER)
        .set_align(FormatAlign::Center)
}

fn label_fmt() -> Format {
    Format::new()
        .set_bold()
        .set_font_color(MUTED)
        .set_border(FormatBorder::Thin)
        .set_border_color(BORDER)
        .set_background_color(BRAND_SOFT)
        .set_align(FormatAlign::VerticalCenter)
}

fn kpi_fmt(big: bool) -> Format {
    let f = Format::new()
        .set_bold()
        .set_border(FormatBorder::Thin)
        .set_border_color(BORDER)
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter);
    if big {
        f.set_font_size(20)
    } else {
        f.set_font_size(14)
    }
}

fn chip_fmt(bg: Color, fg: Color) -> Format {
    Format::new()
        .set_bold()
        .set_font_color(fg)
        .set_background_color(bg)
        .set_border(FormatBorder::Thin)
        .set_border_color(BORDER)
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter)
}

fn note_fmt() -> Format {
    Format::new()
        .set_font_color(MUTED)
        .set_font_size(9)
        .set_align(FormatAlign::VerticalCenter)
}

// ---------- 小工具 ----------

fn clip(s: &str) -> String {
    if s.chars().count() <= CELL_LIMIT {
        return s.to_string();
    }
    let mut out: String = s.chars().take(CELL_LIMIT).collect();
    out.push_str("…（内容过长已截断，完整原文请用「导出 JSON」）");
    out
}

fn verdict_style(v: &str) -> (Color, Color) {
    match v {
        "通过" => (PASS_BG, PASS_FG),
        "未计入" => (WARN_BG, WARN_FG),
        _ => (FAIL_BG, FAIL_FG),
    }
}

fn latency_text(ms: u64) -> String {
    if ms == 0 {
        "—".to_string()
    } else {
        format!("{:.1}", ms as f64 / 1000.0)
    }
}

fn non_empty(s: &str) -> String {
    if s.trim().is_empty() {
        "—".into()
    } else {
        s.to_string()
    }
}

fn conclusion(s: &Summary) -> String {
    if s.total == 0 {
        return "没有结果".into();
    }
    if s.valid == 0 {
        return "全部样本未跑通，无法计算通过率，请检查接口配置与超时设置".into();
    }
    let rate = s.pass_rate * 100.0;
    let band = if rate >= 90.0 {
        "表现良好"
    } else if rate >= 70.0 {
        "基本可用，仍有明显缺陷"
    } else {
        "问题较多，需优先排查主要失败分类"
    };
    let mut out = format!("通过率 {rate:.1}%，{band}");
    if s.excluded > 0 {
        out.push_str(&format!(
            "；另有 {} 条未计入（调用失败/超时/解析失败）",
            s.excluded
        ));
    }
    out
}

/// 写表头 + 列宽 + 冻结 + 表头行高，返回数据起始行。
/// freeze 为 false 时不冻结（概览页的 TOP 表嵌在页面中间，冻结会切断上半部分）。
fn write_table_head(
    ws: &mut Worksheet,
    head_row: u32,
    headers: &[&str],
    widths: &[f64],
    freeze: bool,
) -> Result<u32, String> {
    for (i, (h, w)) in headers.iter().zip(widths.iter()).enumerate() {
        ws.write_string_with_format(head_row, i as u16, *h, &head_fmt())
            .map_err(|e| format!("写入表头失败：{e}"))?;
        ws.set_column_width(i as u16, *w)
            .map_err(|e| format!("设置列宽失败：{e}"))?;
    }
    if freeze {
        ws.set_freeze_panes(head_row + 1, 0)
            .map_err(|e| format!("冻结表头失败：{e}"))?;
    }
    ws.set_row_height(head_row, 24.0)
        .map_err(|e| format!("设置行高失败：{e}"))?;
    Ok(head_row + 1)
}

fn zebra(i: usize) -> Format {
    if i % 2 == 0 {
        cell_fmt()
    } else {
        cell_fmt().set_background_color(ZEBRA)
    }
}

// ---------- 各 sheet ----------

fn write_overview(ws: &mut Worksheet, p: &ReportPayload) -> Result<(), String> {
    ws.set_name("概览").map_err(|e| e.to_string())?;
    ws.set_screen_gridlines(false);

    for (i, w) in [22.0, 16.0, 16.0, 16.0, 16.0, 16.0].iter().enumerate() {
        ws.set_column_width(i as u16, *w).map_err(|e| e.to_string())?;
    }

    // 用用户填的评测名称当报告标题；没填就退回通用名称
    let heading = if p.title.trim().is_empty() {
        "批量评测报告".to_string()
    } else {
        p.title.trim().to_string()
    };
    ws.merge_range(0, 0, 0, 5, &heading, &title_fmt())
        .map_err(|e| e.to_string())?;
    ws.set_row_height(0, 40.0).map_err(|e| e.to_string())?;

    let generated = if p.generated_at.is_empty() {
        String::new()
    } else {
        format!(
            "生成时间 {}",
            p.generated_at.replace('T', " ").replace('Z', "")
        )
    };
    ws.merge_range(1, 0, 1, 5, &generated, &note_fmt())
        .map_err(|e| e.to_string())?;

    let source = {
        let pn = if p.plan_name.trim().is_empty() {
            "默认方案"
        } else {
            p.plan_name.trim()
        };
        let dn = if p.dataset_name.trim().is_empty() {
            "默认数据集"
        } else {
            p.dataset_name.trim()
        };
        format!("方案 {pn} · 数据集 {dn}")
    };
    ws.merge_range(2, 0, 2, 5, &source, &note_fmt())
        .map_err(|e| e.to_string())?;

    // 模型参数：被测模型与评测模型并排，方便对照
    let mut r = 4u32;
    ws.merge_range(r, 0, r, 5, "模型参数", &section_fmt())
        .map_err(|e| e.to_string())?;
    r += 1;
    r = write_table_head(
        ws,
        r,
        &["参数", "被测模型", "评测模型"],
        &[22.0, 20.0, 20.0],
        false,
    )?;

    let t = &p.target_profile;
    let j = &p.judge_profile;
    let num_or_dash = |v: f64| {
        if v.is_finite() && v != 0.0 {
            if v.fract() == 0.0 {
                format!("{}", v as i64)
            } else {
                format!("{v}")
            }
        } else if v == 0.0 {
            "0".to_string()
        } else {
            "—".to_string()
        }
    };
    let text_or_dash = |s: &str| {
        if s.trim().is_empty() {
            "—".to_string()
        } else {
            s.trim().to_string()
        }
    };

    let param_rows: [(&str, String, String); 9] = [
        (
            "协议",
            non_empty(&if t.protocol == "anthropic" { "Anthropic" } else { "OpenAI 兼容" }),
            non_empty(&if j.protocol == "anthropic" { "Anthropic" } else { "OpenAI 兼容" }),
        ),
        ("模型", non_empty(&t.model), non_empty(&j.model)),
        ("上下文长度 (tokens)", num_or_dash(t.context_window), num_or_dash(j.context_window)),
        ("max_tokens", num_or_dash(t.max_tokens), num_or_dash(j.max_tokens)),
        ("temperature", num_or_dash(t.temperature), num_or_dash(j.temperature)),
        ("top_p", num_or_dash(t.top_p), num_or_dash(j.top_p)),
        ("超时 (秒)", num_or_dash(t.timeout_secs), num_or_dash(j.timeout_secs)),
        (
            "高级参数 (extra body)",
            text_or_dash(&crate::storage::redact_sensitive_json(&t.extra_body)),
            text_or_dash(&crate::storage::redact_sensitive_json(&j.extra_body)),
        ),
        (
            "自定义请求头",
            text_or_dash(&crate::storage::redact_sensitive_json(&t.extra_headers)),
            text_or_dash(&crate::storage::redact_sensitive_json(&j.extra_headers)),
        ),
    ];
    for (i, (label, tv, jv)) in param_rows.iter().enumerate() {
        let base = zebra(i);
        ws.write_string_with_format(r, 0, *label, &label_fmt())
            .map_err(|e| e.to_string())?;
        ws.write_string_with_format(r, 1, clip(tv), &base.clone().set_text_wrap())
            .map_err(|e| e.to_string())?;
        ws.write_string_with_format(r, 2, clip(jv), &base.set_text_wrap())
            .map_err(|e| e.to_string())?;
        // 高级参数可能是多行 JSON，给足高度
        let tall = label.contains("extra") || label.contains("请求头");
        ws.set_row_height(r, if tall { 48.0 } else { 18.0 })
            .map_err(|e| e.to_string())?;
        r += 1;
    }
    r += 1;

    // 核心指标
    r += 1;
    ws.merge_range(r, 0, r, 5, "核心指标", &section_fmt())
        .map_err(|e| e.to_string())?;
    r += 1;

    let s = &p.summary;
    let kpi_label = Format::new()
        .set_bold()
        .set_font_color(MUTED)
        .set_align(FormatAlign::Center)
        .set_background_color(BRAND_SOFT);
    ws.write_string_with_format(r, 0, "通过率", &kpi_label)
        .map_err(|e| e.to_string())?;
    let labels = ["有效样本", "通过", "不通过", "未计入", "样本总数"];
    let values = [s.valid, s.passed, s.failed, s.excluded, s.total];
    for (i, (label, value)) in labels.iter().zip(values.iter()).enumerate() {
        let col = (i + 1) as u16;
        ws.write_string_with_format(r, col, *label, &kpi_label)
            .map_err(|e| e.to_string())?;
        ws.write_number_with_format(r + 1, col, *value as f64, &kpi_fmt(false))
            .map_err(|e| e.to_string())?;
    }
    ws.write_number_with_format(r + 1, 0, s.pass_rate, &kpi_fmt(true).set_num_format("0.0%"))
        .map_err(|e| e.to_string())?;
    ws.set_row_height(r + 1, 34.0).map_err(|e| e.to_string())?;
    r += 2;

    ws.merge_range(
        r,
        0,
        r,
        5,
        "通过率 = 通过 ÷ 有效样本；调用失败、模型超时、评测输出解析失败不计入分母，单独列为「未计入」。",
        &note_fmt(),
    )
    .map_err(|e| e.to_string())?;
    r += 2;

    // 其他
    ws.merge_range(r, 0, r, 5, "其他", &section_fmt())
        .map_err(|e| e.to_string())?;
    r += 1;
    let avg = if s.avg_target_latency_ms == 0 {
        "—".to_string()
    } else {
        format!("{:.1} 秒", s.avg_target_latency_ms as f64 / 1000.0)
    };
    let extra: [(&str, String); 3] = [
        ("平均响应耗时", avg),
        ("失败分类数", p.categories.len().to_string()),
        ("结论", conclusion(s)),
    ];
    for (label, value) in extra {
        ws.write_string_with_format(r, 0, label, &label_fmt())
            .map_err(|e| e.to_string())?;
        ws.merge_range(r, 1, r, 5, &value, &cell_fmt())
            .map_err(|e| e.to_string())?;
        ws.set_row_height(r, 20.0).map_err(|e| e.to_string())?;
        r += 1;
    }
    r += 1;

    // 失败分类 TOP
    ws.merge_range(r, 0, r, 5, "失败分类 TOP", &section_fmt())
        .map_err(|e| e.to_string())?;
    r += 1;
    if p.categories.is_empty() {
        ws.merge_range(r, 0, r, 5, "没有失败分类", &note_fmt())
            .map_err(|e| e.to_string())?;
    } else {
        r = write_table_head(ws, r, &["分类", "条数", "占不通过"], &[22.0, 16.0, 16.0], false)?;
        for (i, it) in p.categories.iter().take(8).enumerate() {
            let base = zebra(i);
            ws.write_string_with_format(r, 0, clip(&it.name), &base)
                .map_err(|e| e.to_string())?;
            ws.write_number_with_format(r, 1, it.count as f64, &num_fmt())
                .map_err(|e| e.to_string())?;
            ws.write_number_with_format(r, 2, it.ratio, &pct_fmt())
                .map_err(|e| e.to_string())?;
            r += 1;
        }
    }
    Ok(())
}

fn write_categories(ws: &mut Worksheet, p: &ReportPayload) -> Result<(), String> {
    ws.set_name("错误分类").map_err(|e| e.to_string())?;
    ws.set_screen_gridlines(false);

    ws.merge_range(0, 0, 0, 3, "错误分类分布", &title_fmt())
        .map_err(|e| e.to_string())?;
    ws.set_row_height(0, 30.0).map_err(|e| e.to_string())?;

    let s = &p.summary;
    let mut r = write_table_head(
        ws,
        2,
        &["分类", "条数", "占不通过", "占有效样本"],
        &[38.0, 10.0, 14.0, 14.0],
        true,
    )?;

    if p.categories.is_empty() {
        ws.merge_range(
            r,
            0,
            r,
            3,
            "没有失败分类（未计入项见「未计入通过率」sheet）",
            &note_fmt(),
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }
    for (i, it) in p.categories.iter().enumerate() {
        let base = zebra(i);
        ws.write_string_with_format(r, 0, clip(&it.name), &base)
            .map_err(|e| e.to_string())?;
        ws.write_number_with_format(r, 1, it.count as f64, &num_fmt())
            .map_err(|e| e.to_string())?;
        ws.write_number_with_format(r, 2, it.ratio, &pct_fmt())
            .map_err(|e| e.to_string())?;
        let of_valid = if s.valid > 0 {
            it.count as f64 / s.valid as f64
        } else {
            0.0
        };
        ws.write_number_with_format(r, 3, of_valid, &pct_fmt())
            .map_err(|e| e.to_string())?;
        r += 1;
    }
    Ok(())
}

fn write_rows(ws: &mut Worksheet, p: &ReportPayload) -> Result<(), String> {
    ws.set_name("明细").map_err(|e| e.to_string())?;
    ws.set_screen_gridlines(false);

    let headers = [
        "序号", "结果", "数据组", "分类", "原因", "细节", "被测模型输出", "用户消息",
        "被测耗时(s)", "评测耗时(s)", "输入tokens", "输出tokens", "调用错误", "评测原始输出",
    ];
    let widths = [
        7.0, 9.0, 14.0, 20.0, 40.0, 30.0, 45.0, 40.0, 12.0, 12.0, 11.0, 11.0, 30.0, 40.0,
    ];
    let mut r = write_table_head(ws, 0, &headers, &widths, true)?;
    let last_col = (headers.len() - 1) as u16;
    if !p.rows.is_empty() {
        ws.autofilter(0, 0, (r + p.rows.len() as u32).saturating_sub(1), last_col)
            .map_err(|e| e.to_string())?;
    }

    for (i, row) in p.rows.iter().enumerate() {
        let (bg, fg) = verdict_style(&row.verdict);
        let base = zebra(i);

        ws.write_number_with_format(r, 0, row.index as f64, &base)
            .map_err(|e| e.to_string())?;
        ws.write_string_with_format(r, 1, &row.verdict, &chip_fmt(bg, fg))
            .map_err(|e| e.to_string())?;
        for (col, text) in [
            (2u16, &row.group),
            (3, &row.categories),
            (4, &row.reasons),
            (5, &row.details),
            (6, &row.output),
            (7, &row.input),
        ] {
            // Format 实现的是消费式 setter，循环里要逐个 clone
            ws.write_string_with_format(r, col, clip(text), &base.clone().set_text_wrap())
                .map_err(|e| e.to_string())?;
        }
        ws.write_string_with_format(r, 8, latency_text(row.target_latency_ms), &num_fmt())
            .map_err(|e| e.to_string())?;
        ws.write_string_with_format(r, 9, latency_text(row.judge_latency_ms), &num_fmt())
            .map_err(|e| e.to_string())?;
        match row.prompt_tokens {
            Some(v) => ws.write_number_with_format(r, 10, v as f64, &num_fmt()),
            None => ws.write_string_with_format(r, 10, "—", &num_fmt()),
        }
        .map_err(|e| e.to_string())?;
        match row.completion_tokens {
            Some(v) => ws.write_number_with_format(r, 11, v as f64, &num_fmt()),
            None => ws.write_string_with_format(r, 11, "—", &num_fmt()),
        }
        .map_err(|e| e.to_string())?;
        ws.write_string_with_format(r, 12, clip(&row.target_error), &wrap_fmt())
            .map_err(|e| e.to_string())?;
        ws.write_string_with_format(r, 13, clip(&row.judge_raw), &wrap_fmt())
            .map_err(|e| e.to_string())?;

        // 长文本行给固定高度，避免整页撑成几万像素；具体内容在单元格内换行可看
        ws.set_row_height(r, 30.0).map_err(|e| e.to_string())?;
        r += 1;
    }
    Ok(())
}

// ---------- 对外命令 ----------

/// 生成并保存完整报告。路径由前端文件对话框给出。
#[tauri::command]
pub async fn export_excel_report(path: String, payload: ReportPayload) -> Result<(), String> {
    let mut wb = Workbook::new();
    // 一次性写好后保存进文件：1 万条时前端已经分好了结构，这里不再分块
    write_overview(wb.add_worksheet(), &payload)?;
    write_categories(wb.add_worksheet(), &payload)?;
    write_rows(wb.add_worksheet(), &payload)?;

    let doc_title = if payload.title.trim().is_empty() {
        "批量评测报告".to_string()
    } else {
        payload.title.trim().to_string()
    };
    wb.set_properties(
        &DocProperties::new()
            .set_title(&doc_title)
            .set_subject(if payload.target_profile.model.is_empty() {
                "批量评测"
            } else {
                &payload.target_profile.model
            })
            .set_author("黑猫-自动评测台"),
    );

    wb.save(&path)
        .map_err(|e| format!("保存报告失败：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ReportPayload {
        ReportPayload {
            title: "测试主题".into(),
            plan_name: "客服话术-v3".into(),
            dataset_name: "客服问题.csv".into(),
            generated_at: "2026-11-02T10:00:00Z".into(),
            dataset_rows: 4,
            target_profile: ProfileInfo {
                protocol: "openai".into(),
                model: "gpt-4o".into(),
                context_window: 128000.0,
                max_tokens: 4096.0,
                temperature: 0.7,
                top_p: 1.0,
                timeout_secs: 180.0,
                extra_body: "{\"reasoning_effort\":\"high\"}".into(),
                extra_headers: "".into(),
            },
            judge_profile: ProfileInfo {
                protocol: "anthropic".into(),
                model: "claude-sonnet".into(),
                context_window: 200000.0,
                max_tokens: 2048.0,
                temperature: 0.0,
                top_p: 1.0,
                timeout_secs: 120.0,
                ..Default::default()
            },
            summary: Summary {
                total: 4,
                valid: 3,
                passed: 2,
                failed: 1,
                excluded: 1,
                pass_rate: 2.0 / 3.0,
                avg_target_latency_ms: 1200,
            },
            categories: vec![CatItem { name: "字段缺失".into(), count: 1, ratio: 1.0 }],
            rows: vec![
                ReportRow { index: 1, group: "A".into(), verdict: "通过".into(), ..Default::default() },
                ReportRow { index: 2, group: "A".into(), verdict: "不通过".into(), reasons: "缺字段".into(), ..Default::default() },
                ReportRow { index: 3, group: "B".into(), verdict: "未计入".into(), excluded: true, target_error: "请求超时".into(), ..Default::default() },
                ReportRow { index: 4, group: "B".into(), verdict: "通过".into(), ..Default::default() },
            ],
        }
    }

    /// 生成真实文件，确保三个 sheet 都能落盘且行数对得上
    #[test]
    fn writes_workbook_with_all_sheets() {
        let dir = std::env::temp_dir().join(format!("batch-eval-report-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.xlsx");
        let mut wb = Workbook::new();
        let p = sample();
        write_overview(wb.add_worksheet(), &p).unwrap();
        write_categories(wb.add_worksheet(), &p).unwrap();
        write_rows(wb.add_worksheet(), &p).unwrap();
        wb.save(&path).unwrap();

        let len = std::fs::metadata(&path).unwrap().len();
        assert!(len > 3000, "xlsx 太小，可能没写进去：{len} 字节");
        // 人工验收时设 KEEP_REPORT=1 保留文件
        if std::env::var("KEEP_REPORT").is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        } else {
            println!("report kept at {}", path.display());
        }
    }
}
