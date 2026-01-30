use docx_rs::*;

use super::{ats_optimizer::ATSAnalysis, resume_optimizer::OptimizedResume};

/// Word文档生成器
pub struct DocumentGenerator;

impl DocumentGenerator {
    /// 生成优化后的简历Word文档
    pub async fn generate_optimized_resume(
        optimized: &OptimizedResume,
        ats_analysis: &ATSAnalysis,
        _original_name: &str,
    ) -> Result<Vec<u8>, anyhow::Error> {
        tracing::info!("📝 开始生成优化后的简历文档");

        let mut docx = Docx::new();

        // 1. 标题
        docx = docx.add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text("优化后的简历").size(32).bold())
                .align(AlignmentType::Center),
        );

        // 2. ATS评分展示
        docx = docx.add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text(&format!(
                        "ATS优化评分: {} → {} (+{}分)",
                        ats_analysis.ats_score,
                        optimized.estimated_ats_score,
                        optimized.estimated_ats_score - ats_analysis.ats_score
                    ))
                    .size(24)
                    .color("2E7D32"),
            ),
        );

        docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));

        // 3. 个人简介
        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text("个人简介").size(28).bold()),
        );

        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(&optimized.optimized_summary).size(20)),
        );

        docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));

        // 4. 核心技能
        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text("核心技能").size(28).bold()),
        );

        for skill in &optimized.optimized_skills {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(
                    Run::new()
                        .add_text(&format!("• {}: {}", skill.skill, skill.description))
                        .size(20),
                ),
            );
        }

        docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));

        // 5. 工作经历
        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text("工作经历").size(28).bold()),
        );

        for exp in &optimized.optimized_experience {
            // 公司和职位
            docx = docx.add_paragraph(
                Paragraph::new().add_run(
                    Run::new()
                        .add_text(&format!("{} - {}", exp.company, exp.position))
                        .size(24)
                        .bold(),
                ),
            );

            // 时间
            docx = docx.add_paragraph(
                Paragraph::new().add_run(
                    Run::new()
                        .add_text(&exp.duration)
                        .size(20)
                        .italic()
                        .color("666666"),
                ),
            );

            // 成就（STAR法则）
            for achievement in &exp.achievements {
                docx = docx.add_paragraph(
                    Paragraph::new()
                        .add_run(Run::new().add_text(&format!("  {}", achievement)).size(20)),
                );
            }

            docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));
        }

        // 6. 优化亮点
        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text("优化亮点").size(28).bold()),
        );

        for highlight in &optimized.improvement_highlights {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(
                    Run::new()
                        .add_text(&format!("✓ {}", highlight))
                        .size(20)
                        .color("1976D2"),
                ),
            );
        }

        docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));

        // 7. 新增关键词
        if !optimized.added_keywords.is_empty() {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("新增关键词").size(24).bold()),
            );

            docx = docx.add_paragraph(
                Paragraph::new().add_run(
                    Run::new()
                        .add_text(&optimized.added_keywords.join("、"))
                        .size(20),
                ),
            );
        }

        // 生成文档
        let mut buf = std::io::Cursor::new(Vec::new());
        docx.build().pack(&mut buf)?;

        let result = buf.into_inner();
        tracing::info!("✅ Word文档生成完成，大小: {} bytes", result.len());

        Ok(result)
    }

    /// 生成ATS分析报告
    pub async fn generate_ats_report(
        ats_analysis: &ATSAnalysis,
        file_name: &str,
    ) -> Result<Vec<u8>, anyhow::Error> {
        tracing::info!("📊 开始生成ATS分析报告");

        let mut docx = Docx::new();

        // 标题
        docx = docx.add_paragraph(
            Paragraph::new()
                .add_run(Run::new().add_text("ATS兼容性分析报告").size(32).bold())
                .align(AlignmentType::Center),
        );

        docx = docx.add_paragraph(
            Paragraph::new()
                .add_run(
                    Run::new()
                        .add_text(&format!("文件: {}", file_name))
                        .size(20)
                        .italic(),
                )
                .align(AlignmentType::Center),
        );

        docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));

        // 总体评分
        docx = docx.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text("总体评分").size(28).bold()),
        );

        let score_color = if ats_analysis.ats_score >= 80 {
            "2E7D32"
        } else if ats_analysis.ats_score >= 60 {
            "F57C00"
        } else {
            "D32F2F"
        };

        docx = docx.add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text(&format!("ATS评分: {}/100", ats_analysis.ats_score))
                    .size(24)
                    .bold()
                    .color(score_color),
            ),
        );

        docx = docx.add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text(&format!("关键词密度: {}/100", ats_analysis.keyword_density))
                    .size(20),
            ),
        );

        docx = docx.add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text(&format!("结构评分: {}/100", ats_analysis.structure_score))
                    .size(20),
            ),
        );

        docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));

        // 缺失关键词
        if !ats_analysis.missing_keywords.is_empty() {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("缺失关键词").size(28).bold()),
            );

            for keyword in &ats_analysis.missing_keywords {
                let importance_color = match keyword.importance.as_str() {
                    "high" => "D32F2F",
                    "medium" => "F57C00",
                    _ => "666666",
                };

                docx = docx.add_paragraph(
                    Paragraph::new().add_run(
                        Run::new()
                            .add_text(&format!(
                                "• {} [{}] - {}",
                                keyword.keyword, keyword.importance, keyword.reason
                            ))
                            .size(20)
                            .color(importance_color),
                    ),
                );
            }

            docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));
        }

        // 关键词优化建议
        if !ats_analysis.keyword_suggestions.is_empty() {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("关键词优化建议").size(28).bold()),
            );

            for suggestion in &ats_analysis.keyword_suggestions {
                docx = docx.add_paragraph(
                    Paragraph::new().add_run(
                        Run::new()
                            .add_text(&format!("当前: {}", suggestion.current))
                            .size(20),
                    ),
                );

                docx = docx.add_paragraph(
                    Paragraph::new().add_run(
                        Run::new()
                            .add_text(&format!("建议: {}", suggestion.suggested))
                            .size(20)
                            .color("1976D2"),
                    ),
                );

                docx = docx.add_paragraph(
                    Paragraph::new().add_run(
                        Run::new()
                            .add_text(&format!("原因: {}", suggestion.reason))
                            .size(18)
                            .italic(),
                    ),
                );

                docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));
            }
        }

        // 结构问题
        if !ats_analysis.structure_issues.is_empty() {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("结构优化建议").size(28).bold()),
            );

            for issue in &ats_analysis.structure_issues {
                docx = docx.add_paragraph(
                    Paragraph::new().add_run(
                        Run::new()
                            .add_text(&format!("问题: {}", issue.issue))
                            .size(20),
                    ),
                );

                docx = docx.add_paragraph(
                    Paragraph::new().add_run(
                        Run::new()
                            .add_text(&format!("建议: {}", issue.suggestion))
                            .size(20)
                            .color("1976D2"),
                    ),
                );

                docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text("")));
            }
        }

        // 总体建议
        if !ats_analysis.overall_suggestions.is_empty() {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("总体建议").size(28).bold()),
            );

            for suggestion in &ats_analysis.overall_suggestions {
                docx = docx.add_paragraph(
                    Paragraph::new()
                        .add_run(Run::new().add_text(&format!("✓ {}", suggestion)).size(20)),
                );
            }
        }

        // 生成文档
        let mut buf = std::io::Cursor::new(Vec::new());
        docx.build().pack(&mut buf)?;

        let result = buf.into_inner();
        tracing::info!("✅ ATS报告生成完成，大小: {} bytes", result.len());

        Ok(result)
    }
}
