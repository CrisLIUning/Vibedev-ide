use gpui::{Action as _, App};
use itertools::Itertools as _;
use settings::{
    AudioInputDeviceName, AudioOutputDeviceName, LanguageSettingsContent, SemanticTokens,
    SettingsContent,
};
use std::sync::{Arc, OnceLock};
use strum::{EnumMessage, IntoDiscriminant as _, VariantArray};
use theme::SystemAppearance;
use ui::IntoElement;

use crate::{
    ActionLink, DynamicItem, PROJECT, SettingField, SettingItem, SettingsFieldMetadata,
    SettingsPage, SettingsPageItem, SubPageLink, USER, active_language, all_language_names,
    pages::{
        open_audio_test_window, render_edit_prediction_setup_page, render_skills_setup_page,
        render_tool_permissions_setup_page,
    },
};

const DEFAULT_STRING: String = String::new();
/// A default empty string reference. Useful in `pick` functions for cases either in dynamic item fields, or when dealing with `settings::Maybe`
/// to avoid the "NO DEFAULT" case.
const DEFAULT_EMPTY_STRING: Option<&String> = Some(&DEFAULT_STRING);

const DEFAULT_AUDIO_OUTPUT: AudioOutputDeviceName = AudioOutputDeviceName(None);
const DEFAULT_EMPTY_AUDIO_OUTPUT: Option<&AudioOutputDeviceName> = Some(&DEFAULT_AUDIO_OUTPUT);
const DEFAULT_AUDIO_INPUT: AudioInputDeviceName = AudioInputDeviceName(None);
const DEFAULT_EMPTY_AUDIO_INPUT: Option<&AudioInputDeviceName> = Some(&DEFAULT_AUDIO_INPUT);

macro_rules! concat_sections {
    (@vec, $($arr:expr),+ $(,)?) => {{
        let total_len = 0_usize $(+ $arr.len())+;
        let mut out = Vec::with_capacity(total_len);

        $(
            out.extend($arr);
        )+

        out
    }};

    ($($arr:expr),+ $(,)?) => {{
        let total_len = 0_usize $(+ $arr.len())+;

        let mut out: Box<[std::mem::MaybeUninit<_>]> = Box::new_uninit_slice(total_len);

        let mut index = 0usize;
        $(
            let array = $arr;
            for item in array {
                out[index].write(item);
                index += 1;
            }
        )+

        debug_assert_eq!(index, total_len);

        // SAFETY: we wrote exactly `total_len` elements.
        unsafe { out.assume_init() }
    }};
}

pub(crate) fn settings_data(cx: &App) -> Vec<SettingsPage> {
    let mut pages = vec![
        general_page(cx),
        appearance_page(),
        keymap_page(),
        editor_page(),
        languages_and_tools_page(cx),
        search_and_files_page(),
        window_and_layout_page(),
        panels_page(),
        debugger_page(),
        terminal_page(),
        version_control_page(),
        collaboration_page(),
        ai_page(cx),
        network_page(),
    ];

    use feature_flags::FeatureFlagAppExt as _;
    if cx.is_staff() || cfg!(debug_assertions) {
        pages.push(developer_page());
    }

    pages
}

fn developer_page() -> SettingsPage {
    SettingsPage {
        title: "开发者",
        items: Box::new([
            SettingsPageItem::SectionHeader("功能开关"),
            SettingsPageItem::SubPageLink(SubPageLink {
                title: "功能开关".into(),
                r#type: Default::default(),
                description: None,
                json_path: Some("feature_flags"),
                in_json: true,
                files: USER,
                render: crate::pages::render_feature_flags_page,
            }),
            SettingsPageItem::SectionHeader("性能追踪"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "性能分析器",
                description: "收集前台和后台执行器任务的计时数据,以便通过 `zed: 打开 performance profiler` 进行检查。可能导致内存使用增加。",
                field: Box::new(SettingField {
                    json_path: Some("instrumentation.performance_profiler.enabled"),
                    pick: |settings_content| {
                        settings_content
                            .instrumentation
                            .as_ref()
                            .and_then(|i| i.performance_profiler.as_ref())
                            .and_then(|p| p.enabled.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .instrumentation
                            .get_or_insert_default()
                            .performance_profiler
                            .get_or_insert_default()
                            .enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]),
    }
}

fn general_page(cx: &App) -> SettingsPage {
    fn general_settings_section(_cx: &App) -> Vec<SettingsPageItem> {
        vec![
            SettingsPageItem::SectionHeader("常规设置"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "无标签页关闭时",
                description: "在没有标签页时使用“关闭活动项目”操作时的处理方式。",
                field: Box::new(SettingField {
                    json_path: Some("when_closing_with_no_tabs"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .when_closing_with_no_tabs
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.when_closing_with_no_tabs = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "最后一个窗口关闭时",
                description: "当最后一个窗口关闭时的处理方式。",
                field: Box::new(SettingField {
                    json_path: Some("on_last_window_closed"),
                    pick: |settings_content| {
                        settings_content.workspace.on_last_window_closed.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.on_last_window_closed = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用系统路径提示",
                description: "使用原生操作系统对话框进行“打开”和“另存为”。",
                field: Box::new(SettingField {
                    json_path: Some("use_system_path_prompts"),
                    pick: |settings_content| {
                        settings_content.workspace.use_system_path_prompts.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.use_system_path_prompts = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用系统提示",
                description: "使用原生操作系统对话框进行确认。",
                field: Box::new(SettingField {
                    json_path: Some("use_system_prompts"),
                    pick: |settings_content| settings_content.workspace.use_system_prompts.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.workspace.use_system_prompts = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "编辑私有值",
                description: "隐藏私有文件中变量的值。",
                field: Box::new(SettingField {
                    json_path: Some("redact_private_values"),
                    pick: |settings_content| settings_content.editor.redact_private_values.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.redact_private_values = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "私有文件",
                description: "用于匹配文件路径以确定文件是否为私有的 Glob 模式。",
                field: Box::new(
                    SettingField {
                        json_path: Some("worktree.private_files"),
                        pick: |settings_content| {
                            settings_content.project.worktree.private_files.as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content.project.worktree.private_files = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "CLI 默认打开行为",
                description: "未指定标志时 `zed <path>` 打开目录的方式。",
                field: Box::new(SettingField {
                    json_path: Some("cli_default_open_behavior"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .cli_default_open_behavior
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.cli_default_open_behavior = value;
                    },
                }),
                metadata: Some(Box::new(SettingsFieldMetadata {
                    should_do_titlecase: Some(false),
                    ..Default::default()
                })),
                files: USER,
            }),
        ]
    }
    fn security_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("安全"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "默认信任所有项目",
                description: "When opening VibeDev, avoid Restricted Mode by auto-trusting all projects, enabling use of all features without having to give permission to each new project.",
                field: Box::new(SettingField {
                    json_path: Some("session.trust_all_projects"),
                    pick: |settings_content| {
                        settings_content
                            .session
                            .as_ref()
                            .and_then(|session| session.trust_all_worktrees.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .session
                            .get_or_insert_default()
                            .trust_all_worktrees = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn workspace_restoration_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("工作区恢复"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "恢复未保存的缓冲区",
                description: "重启时是否恢复未保存的缓冲区。",
                field: Box::new(SettingField {
                    json_path: Some("session.restore_unsaved_buffers"),
                    pick: |settings_content| {
                        settings_content
                            .session
                            .as_ref()
                            .and_then(|session| session.restore_unsaved_buffers.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .session
                            .get_or_insert_default()
                            .restore_unsaved_buffers = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启动时恢复",
                description: "What to restore from the previous session when opening VibeDev.",
                field: Box::new(SettingField {
                    json_path: Some("restore_on_startup"),
                    pick: |settings_content| settings_content.workspace.restore_on_startup.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.workspace.restore_on_startup = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn scoped_settings_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("作用域设置"),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "预览频道",
                description: "Which settings should be activated only in Preview build of VibeDev.",
                field: Box::new(
                    SettingField {
                        json_path: Some("preview_channel_settings"),
                        pick: |settings_content| Some(settings_content),
                        write: |_settings_content, _value, _| {},
                    }
                    .unimplemented(),
                ),
                metadata: None,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "设置配置文件",
                description: "任意数量的设置配置文件,它们将临时应用于您现有的用户设置之上。",
                field: Box::new(
                    SettingField {
                        json_path: Some("settings_profiles"),
                        pick: |settings_content| Some(settings_content),
                        write: |_settings_content, _value, _| {},
                    }
                    .unimplemented(),
                ),
                metadata: None,
            }),
        ]
    }

    fn privacy_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("隐私"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "遥测诊断",
                description: "发送调试信息(如崩溃报告)。",
                field: Box::new(SettingField {
                    json_path: Some("telemetry.diagnostics"),
                    pick: |settings_content| {
                        settings_content
                            .telemetry
                            .as_ref()
                            .and_then(|telemetry| telemetry.diagnostics.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .telemetry
                            .get_or_insert_default()
                            .diagnostics = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "遥测指标",
                description: "Send anonymized usage data like what languages you're using VibeDev with.",
                field: Box::new(SettingField {
                    json_path: Some("telemetry.metrics"),
                    pick: |settings_content| {
                        settings_content
                            .telemetry
                            .as_ref()
                            .and_then(|telemetry| telemetry.metrics.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content.telemetry.get_or_insert_default().metrics = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn auto_update_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("自动更新"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动更新",
                description: "是否自动检查更新。",
                field: Box::new(SettingField {
                    json_path: Some("auto_update"),
                    pick: |settings_content| settings_content.auto_update.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.auto_update = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    SettingsPage {
        title: "常规",
        items: concat_sections!(
            @vec,
            general_settings_section(cx),
            security_section(),
            workspace_restoration_section(),
            scoped_settings_section(),
            privacy_section(),
            auto_update_section(),
        )
        .into(),
    }
}

fn appearance_page() -> SettingsPage {
    fn theme_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("主题"),
            SettingsPageItem::DynamicItem(DynamicItem {
                discriminant: SettingItem {
                    files: USER,
                    title: "主题模式",
                    description: "选择静态、固定的主题,或根据外观和浅色/深色模式动态选择主题。",
                    field: Box::new(SettingField {
                        json_path: Some("theme$"),
                        pick: |settings_content| {
                            Some(&dynamic_variants::<settings::ThemeSelection>()[
                                settings_content
                                    .theme
                                    .theme
                                    .as_ref()?
                                    .discriminant() as usize])
                        },
                        write: |settings_content, value, app: &App| {
                            let Some(value) = value else {
                                settings_content.theme.theme = None;
                                return;
                            };
                            let settings_value = settings_content.theme.theme.get_or_insert_default();
                            *settings_value = match value {
                                settings::ThemeSelectionDiscriminants::Static => {
                                    let name = match settings_value {
                                        settings::ThemeSelection::Static(_) => return,
                                        settings::ThemeSelection::Dynamic { mode, light, dark } => {
                                            match mode {
                                                theme_settings::ThemeAppearanceMode::Light => light.clone(),
                                                theme_settings::ThemeAppearanceMode::Dark => dark.clone(),
                                                theme_settings::ThemeAppearanceMode::System => {
                                                    if SystemAppearance::global(app).is_light() {
                                                        light.clone()
                                                    } else {
                                                        dark.clone()
                                                    }
                                                }
                                            }
                                        },
                                    };
                                    settings::ThemeSelection::Static(name)
                                },
                                settings::ThemeSelectionDiscriminants::Dynamic => {
                                    let static_name = match settings_value {
                                        settings::ThemeSelection::Static(theme_name) => theme_name.clone(),
                                        settings::ThemeSelection::Dynamic {..} => return,
                                    };

                                    settings::ThemeSelection::Dynamic {
                                        mode: settings::ThemeAppearanceMode::System,
                                        light: static_name.clone(),
                                        dark: static_name,
                                    }
                                },
                            };
                        },
                    }),
                    metadata: None,
                },
                pick_discriminant: |settings_content| {
                    Some(settings_content.theme.theme.as_ref()?.discriminant() as usize)
                },
                fields: dynamic_variants::<settings::ThemeSelection>().into_iter().map(|variant| {
                    match variant {
                        settings::ThemeSelectionDiscriminants::Static => vec![
                            SettingItem {
                                files: USER,
                                title: "主题名称",
                                description: "您所选主题的名称。",
                                field: Box::new(SettingField {
                                    json_path: Some("theme"),
                                    pick: |settings_content| {
                                        match settings_content.theme.theme.as_ref() {
                                            Some(settings::ThemeSelection::Static(name)) => Some(name),
                                            _ => None
                                        }
                                    },
                                    write: |settings_content, value, _| {
                                        let Some(value) = value else {
                                            return;
                                        };
                                        match settings_content
                                            .theme
                                            .theme.get_or_insert_default() {
                                                settings::ThemeSelection::Static(theme_name) => *theme_name = value,
                                                _ => return
                                            }
                                    },
                                }),
                                metadata: None,
                            }
                        ],
                        settings::ThemeSelectionDiscriminants::Dynamic => vec![
                            SettingItem {
                                files: USER,
                                title: "模式",
                                description: "选择使用选定的浅色或深色主题,还是跟随操作系统的外观配置。",
                                field: Box::new(SettingField {
                                    json_path: Some("theme.mode"),
                                    pick: |settings_content| {
                                        match settings_content.theme.theme.as_ref() {
                                            Some(settings::ThemeSelection::Dynamic { mode, ..}) => Some(mode),
                                            _ => None
                                        }
                                    },
                                    write: |settings_content, value, _| {
                                        let Some(value) = value else {
                                            return;
                                        };
                                        match settings_content
                                            .theme
                                            .theme.get_or_insert_default() {
                                                settings::ThemeSelection::Dynamic{ mode, ..} => *mode = value,
                                                _ => return
                                            }
                                    },
                                }),
                                metadata: None,
                            },
                            SettingItem {
                                files: USER,
                                title: "浅色主题",
                                description: "当模式设置为浅色,或模式设置为系统且系统处于浅色模式时使用的主题。",
                                field: Box::new(SettingField {
                                    json_path: Some("theme.light"),
                                    pick: |settings_content| {
                                        match settings_content.theme.theme.as_ref() {
                                            Some(settings::ThemeSelection::Dynamic { light, ..}) => Some(light),
                                            _ => None
                                        }
                                    },
                                    write: |settings_content, value, _| {
                                        let Some(value) = value else {
                                            return;
                                        };
                                        match settings_content
                                            .theme
                                            .theme.get_or_insert_default() {
                                                settings::ThemeSelection::Dynamic{ light, ..} => *light = value,
                                                _ => return
                                            }
                                    },
                                }),
                                metadata: None,
                            },
                            SettingItem {
                                files: USER,
                                title: "深色主题",
                                description: "当模式设置为深色,或模式设置为系统且系统处于深色模式时使用的主题。",
                                field: Box::new(SettingField {
                                    json_path: Some("theme.dark"),
                                    pick: |settings_content| {
                                        match settings_content.theme.theme.as_ref() {
                                            Some(settings::ThemeSelection::Dynamic { dark, ..}) => Some(dark),
                                            _ => None
                                        }
                                    },
                                    write: |settings_content, value, _| {
                                        let Some(value) = value else {
                                            return;
                                        };
                                        match settings_content
                                            .theme
                                            .theme.get_or_insert_default() {
                                                settings::ThemeSelection::Dynamic{ dark, ..} => *dark = value,
                                                _ => return
                                            }
                                    },
                                }),
                                metadata: None,
                            }
                        ],
                    }
                }).collect(),
            }),
            SettingsPageItem::DynamicItem(DynamicItem {
                discriminant: SettingItem {
                    files: USER,
                    title: "图标主题",
                    description: "The custom set of icons VibeDev will associate with files and directories.",
                    field: Box::new(SettingField {
                        json_path: Some("icon_theme$"),
                        pick: |settings_content| {
                            Some(&dynamic_variants::<settings::IconThemeSelection>()[
                                settings_content
                                    .theme
                                    .icon_theme
                                    .as_ref()?
                                    .discriminant() as usize])
                        },
                        write: |settings_content, value, app| {
                            let Some(value) = value else {
                                settings_content.theme.icon_theme = None;
                                return;
                            };
                            let settings_value = settings_content.theme.icon_theme.get_or_insert_with(|| {
                                settings::IconThemeSelection::Static(settings::IconThemeName(theme::default_icon_theme().name.clone().into()))
                            });
                            *settings_value = match value {
                                settings::IconThemeSelectionDiscriminants::Static => {
                                    let name = match settings_value {
                                        settings::IconThemeSelection::Static(_) => return,
                                        settings::IconThemeSelection::Dynamic { mode, light, dark } => {
                                            match mode {
                                                theme_settings::ThemeAppearanceMode::Light => light.clone(),
                                                theme_settings::ThemeAppearanceMode::Dark => dark.clone(),
                                                theme_settings::ThemeAppearanceMode::System => {
                                                    if SystemAppearance::global(app).is_light() {
                                                        light.clone()
                                                    } else {
                                                        dark.clone()
                                                    }
                                                }
                                            }
                                        },
                                    };
                                    settings::IconThemeSelection::Static(name)
                                },
                                settings::IconThemeSelectionDiscriminants::Dynamic => {
                                    let static_name = match settings_value {
                                        settings::IconThemeSelection::Static(theme_name) => theme_name.clone(),
                                        settings::IconThemeSelection::Dynamic {..} => return,
                                    };

                                    settings::IconThemeSelection::Dynamic {
                                        mode: settings::ThemeAppearanceMode::System,
                                        light: static_name.clone(),
                                        dark: static_name,
                                    }
                                },
                            };
                        },
                    }),
                    metadata: None,
                },
                pick_discriminant: |settings_content| {
                    Some(settings_content.theme.icon_theme.as_ref()?.discriminant() as usize)
                },
                fields: dynamic_variants::<settings::IconThemeSelection>().into_iter().map(|variant| {
                    match variant {
                        settings::IconThemeSelectionDiscriminants::Static => vec![
                            SettingItem {
                                files: USER,
                                title: "图标主题名称",
                                description: "您所选图标主题的名称。",
                                field: Box::new(SettingField {
                                    json_path: Some("icon_theme$string"),
                                    pick: |settings_content| {
                                        match settings_content.theme.icon_theme.as_ref() {
                                            Some(settings::IconThemeSelection::Static(name)) => Some(name),
                                            _ => None
                                        }
                                    },
                                    write: |settings_content, value, _| {
                                        let Some(value) = value else {
                                            return;
                                        };
                                        match settings_content
                                            .theme
                                            .icon_theme.as_mut() {
                                                Some(settings::IconThemeSelection::Static(theme_name)) => *theme_name = value,
                                                _ => return
                                            }
                                    },
                                }),
                                metadata: None,
                            }
                        ],
                        settings::IconThemeSelectionDiscriminants::Dynamic => vec![
                            SettingItem {
                                files: USER,
                                title: "模式",
                                description: "选择使用选定的浅色或深色图标主题,还是跟随操作系统的外观配置。",
                                field: Box::new(SettingField {
                                    json_path: Some("icon_theme"),
                                    pick: |settings_content| {
                                        match settings_content.theme.icon_theme.as_ref() {
                                            Some(settings::IconThemeSelection::Dynamic { mode, ..}) => Some(mode),
                                            _ => None
                                        }
                                    },
                                    write: |settings_content, value, _| {
                                        let Some(value) = value else {
                                            return;
                                        };
                                        match settings_content
                                            .theme
                                            .icon_theme.as_mut() {
                                                Some(settings::IconThemeSelection::Dynamic{ mode, ..}) => *mode = value,
                                                _ => return
                                            }
                                    },
                                }),
                                metadata: None,
                            },
                            SettingItem {
                                files: USER,
                                title: "浅色图标主题",
                                description: "当模式设置为浅色,或模式设置为系统且系统处于浅色模式时使用的图标主题。",
                                field: Box::new(SettingField {
                                    json_path: Some("icon_theme.light"),
                                    pick: |settings_content| {
                                        match settings_content.theme.icon_theme.as_ref() {
                                            Some(settings::IconThemeSelection::Dynamic { light, ..}) => Some(light),
                                            _ => None
                                        }
                                    },
                                    write: |settings_content, value, _| {
                                        let Some(value) = value else {
                                            return;
                                        };
                                        match settings_content
                                            .theme
                                            .icon_theme.as_mut() {
                                                Some(settings::IconThemeSelection::Dynamic{ light, ..}) => *light = value,
                                                _ => return
                                            }
                                    },
                                }),
                                metadata: None,
                            },
                            SettingItem {
                                files: USER,
                                title: "深色图标主题",
                                description: "当模式设置为深色,或模式设置为系统且系统处于深色模式时使用的图标主题。",
                                field: Box::new(SettingField {
                                    json_path: Some("icon_theme.dark"),
                                    pick: |settings_content| {
                                        match settings_content.theme.icon_theme.as_ref() {
                                            Some(settings::IconThemeSelection::Dynamic { dark, ..}) => Some(dark),
                                            _ => None
                                        }
                                    },
                                    write: |settings_content, value, _| {
                                        let Some(value) = value else {
                                            return;
                                        };
                                        match settings_content
                                            .theme
                                            .icon_theme.as_mut() {
                                                Some(settings::IconThemeSelection::Dynamic{ dark, ..}) => *dark = value,
                                                _ => return
                                            }
                                    },
                                }),
                                metadata: None,
                            }
                        ],
                    }
                }).collect(),
            }),
        ]
    }

    fn buffer_font_section() -> [SettingsPageItem; 7] {
        [
            SettingsPageItem::SectionHeader("缓冲区字体"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体系列",
                description: "编辑器文本的字体系列。",
                field: Box::new(SettingField {
                    json_path: Some("buffer_font_family"),
                    pick: |settings_content| settings_content.theme.buffer_font_family.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.theme.buffer_font_family = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体大小",
                description: "编辑器文本的字体大小。",
                field: Box::new(SettingField {
                    json_path: Some("buffer_font_size"),
                    pick: |settings_content| settings_content.theme.buffer_font_size.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.theme.buffer_font_size = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体粗细",
                description: "编辑器文本的字体粗细 (100-900)。",
                field: Box::new(SettingField {
                    json_path: Some("buffer_font_weight"),
                    pick: |settings_content| settings_content.theme.buffer_font_weight.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.theme.buffer_font_weight = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::DynamicItem(DynamicItem {
                discriminant: SettingItem {
                    files: USER,
                    title: "行高",
                    description: "编辑器文本的行高。",
                    field: Box::new(SettingField {
                        json_path: Some("buffer_line_height$"),
                        pick: |settings_content| {
                            Some(
                                &dynamic_variants::<settings::BufferLineHeight>()[settings_content
                                    .theme
                                    .buffer_line_height
                                    .as_ref()?
                                    .discriminant()
                                    as usize],
                            )
                        },
                        write: |settings_content, value, _| {
                            let Some(value) = value else {
                                settings_content.theme.buffer_line_height = None;
                                return;
                            };
                            let settings_value = settings_content
                                .theme
                                .buffer_line_height
                                .get_or_insert_with(|| settings::BufferLineHeight::default());
                            *settings_value = match value {
                                settings::BufferLineHeightDiscriminants::Comfortable => {
                                    settings::BufferLineHeight::Comfortable
                                }
                                settings::BufferLineHeightDiscriminants::Standard => {
                                    settings::BufferLineHeight::Standard
                                }
                                settings::BufferLineHeightDiscriminants::Custom => {
                                    let custom_value =
                                        theme_settings::BufferLineHeight::from(*settings_value)
                                            .value();
                                    settings::BufferLineHeight::Custom(custom_value)
                                }
                            };
                        },
                    }),
                    metadata: None,
                },
                pick_discriminant: |settings_content| {
                    Some(
                        settings_content
                            .theme
                            .buffer_line_height
                            .as_ref()?
                            .discriminant() as usize,
                    )
                },
                fields: dynamic_variants::<settings::BufferLineHeight>()
                    .into_iter()
                    .map(|variant| match variant {
                        settings::BufferLineHeightDiscriminants::Comfortable => vec![],
                        settings::BufferLineHeightDiscriminants::Standard => vec![],
                        settings::BufferLineHeightDiscriminants::Custom => vec![SettingItem {
                            files: USER,
                            title: "自定义行高",
                            description: "自定义行高值(必须至少为 1.0)。",
                            field: Box::new(SettingField {
                                json_path: Some("buffer_line_height"),
                                pick: |settings_content| match settings_content
                                    .theme
                                    .buffer_line_height
                                    .as_ref()
                                {
                                    Some(settings::BufferLineHeight::Custom(value)) => Some(value),
                                    _ => None,
                                },
                                write: |settings_content, value, _| {
                                    let Some(value) = value else {
                                        return;
                                    };
                                    match settings_content.theme.buffer_line_height.as_mut() {
                                        Some(settings::BufferLineHeight::Custom(line_height)) => {
                                            *line_height = f32::max(value, 1.0)
                                        }
                                        _ => return,
                                    }
                                },
                            }),
                            metadata: None,
                        }],
                    })
                    .collect(),
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "字体特性",
                description: "在文本缓冲区中渲染时启用的 OpenType 特性。",
                field: Box::new(
                    SettingField {
                        json_path: Some("buffer_font_features"),
                        pick: |settings_content| {
                            settings_content.theme.buffer_font_features.as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content.theme.buffer_font_features = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "字体回退",
                description: "在文本缓冲区中渲染时使用的字体回退。",
                field: Box::new(
                    SettingField {
                        json_path: Some("buffer_font_fallbacks"),
                        pick: |settings_content| {
                            settings_content.theme.buffer_font_fallbacks.as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content.theme.buffer_font_fallbacks = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
            }),
        ]
    }

    fn ui_font_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("UI 字体"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体系列",
                description: "UI 元素的字体系列。",
                field: Box::new(SettingField {
                    json_path: Some("ui_font_family"),
                    pick: |settings_content| settings_content.theme.ui_font_family.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.theme.ui_font_family = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体大小",
                description: "UI 元素的字体大小。",
                field: Box::new(SettingField {
                    json_path: Some("ui_font_size"),
                    pick: |settings_content| settings_content.theme.ui_font_size.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.theme.ui_font_size = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体粗细",
                description: "UI 元素的字体粗细 (100-900)。",
                field: Box::new(SettingField {
                    json_path: Some("ui_font_weight"),
                    pick: |settings_content| settings_content.theme.ui_font_weight.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.theme.ui_font_weight = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "字体特性",
                description: "在 UI 元素中渲染时启用的 OpenType 特性。",
                field: Box::new(
                    SettingField {
                        json_path: Some("ui_font_features"),
                        pick: |settings_content| settings_content.theme.ui_font_features.as_ref(),
                        write: |settings_content, value, _| {
                            settings_content.theme.ui_font_features = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "字体回退",
                description: "在 UI 中渲染时使用的字体回退。",
                field: Box::new(
                    SettingField {
                        json_path: Some("ui_font_fallbacks"),
                        pick: |settings_content| settings_content.theme.ui_font_fallbacks.as_ref(),
                        write: |settings_content, value, _| {
                            settings_content.theme.ui_font_fallbacks = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
            }),
        ]
    }

    fn agent_panel_font_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("助手面板字体"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "UI 字体大小",
                description: "助手面板中助手响应文本的字体大小。回退到常规 UI 字体大小。",
                field: Box::new(SettingField {
                    json_path: Some("agent_ui_font_size"),
                    pick: |settings_content| {
                        settings_content
                            .theme
                            .agent_ui_font_size
                            .as_ref()
                            .or(settings_content.theme.ui_font_size.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content.theme.agent_ui_font_size = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "缓冲区字体大小",
                description: "助手面板中用户消息文本的字体大小。",
                field: Box::new(SettingField {
                    json_path: Some("agent_buffer_font_size"),
                    pick: |settings_content| {
                        settings_content
                            .theme
                            .agent_buffer_font_size
                            .as_ref()
                            .or(settings_content.theme.buffer_font_size.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content.theme.agent_buffer_font_size = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn text_rendering_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("文本渲染"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文本渲染模式",
                description: "要使用的文本渲染模式。",
                field: Box::new(SettingField {
                    json_path: Some("text_rendering_mode"),
                    pick: |settings_content| {
                        settings_content.workspace.text_rendering_mode.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.text_rendering_mode = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn cursor_section() -> [SettingsPageItem; 5] {
        [
            SettingsPageItem::SectionHeader("光标"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "多光标修饰键",
                description: "用于添加多个光标的修饰键。",
                field: Box::new(SettingField {
                    json_path: Some("multi_cursor_modifier"),
                    pick: |settings_content| settings_content.editor.multi_cursor_modifier.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.multi_cursor_modifier = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标闪烁",
                description: "光标在编辑器中是否闪烁。",
                field: Box::new(SettingField {
                    json_path: Some("cursor_blink"),
                    pick: |settings_content| settings_content.editor.cursor_blink.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.cursor_blink = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标形状",
                description: "编辑器的光标形状。",
                field: Box::new(SettingField {
                    json_path: Some("cursor_shape"),
                    pick: |settings_content| settings_content.editor.cursor_shape.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.cursor_shape = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "隐藏鼠标",
                description: "何时隐藏鼠标光标。",
                field: Box::new(SettingField {
                    json_path: Some("hide_mouse"),
                    pick: |settings_content| settings_content.hide_mouse.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.hide_mouse = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn highlighting_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("高亮"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "不必要代码淡化",
                description: "淡化未使用代码的程度 (0.0 - 0.9)。",
                field: Box::new(SettingField {
                    json_path: Some("unnecessary_code_fade"),
                    pick: |settings_content| settings_content.theme.unnecessary_code_fade.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.theme.unnecessary_code_fade = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "当前行高亮",
                description: "如何高亮当前行。",
                field: Box::new(SettingField {
                    json_path: Some("current_line_highlight"),
                    pick: |settings_content| {
                        settings_content.editor.current_line_highlight.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.current_line_highlight = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "选区高亮",
                description: "高亮显示所选文本的所有出现位置。",
                field: Box::new(SettingField {
                    json_path: Some("selection_highlight"),
                    pick: |settings_content| settings_content.editor.selection_highlight.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.selection_highlight = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "圆角选区",
                description: "文本选中区域是否使用圆角。",
                field: Box::new(SettingField {
                    json_path: Some("rounded_selection"),
                    pick: |settings_content| settings_content.editor.rounded_selection.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.rounded_selection = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "高亮最小对比度",
                description: "在高亮背景上渲染文本时保持的最小 APCA 感知对比度。",
                field: Box::new(SettingField {
                    json_path: Some("minimum_contrast_for_highlights"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .minimum_contrast_for_highlights
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.minimum_contrast_for_highlights = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn guides_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("辅助线"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示自动换行辅助线",
                description: "显示自动换行辅助线(垂直标尺)。",
                field: Box::new(SettingField {
                    json_path: Some("show_wrap_guides"),
                    pick: |settings_content| {
                        settings_content
                            .project
                            .all_languages
                            .defaults
                            .show_wrap_guides
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project
                            .all_languages
                            .defaults
                            .show_wrap_guides = value;
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            // todo(settings_ui): This needs a custom component
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动换行辅助线",
                description: "显示自动换行辅助线的字符计数位置。",
                field: Box::new(
                    SettingField {
                        json_path: Some("wrap_guides"),
                        pick: |settings_content| {
                            settings_content
                                .project
                                .all_languages
                                .defaults
                                .wrap_guides
                                .as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content.project.all_languages.defaults.wrap_guides = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    let items: Box<[SettingsPageItem]> = concat_sections!(
        theme_section(),
        buffer_font_section(),
        ui_font_section(),
        agent_panel_font_section(),
        text_rendering_section(),
        cursor_section(),
        highlighting_section(),
        guides_section(),
    );

    SettingsPage {
        title: "外观",
        items,
    }
}

fn keymap_page() -> SettingsPage {
    fn keybindings_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("键位绑定"),
            SettingsPageItem::ActionLink(ActionLink {
                title: "编辑键位绑定".into(),
                description: Some("在键位映射编辑器中自定义键位绑定。".into()),
                button_text: "打开键位映射".into(),
                on_click: Arc::new(|settings_window, window, cx| {
                    let Some(original_window) = settings_window.original_window else {
                        return;
                    };
                    original_window
                        .update(cx, |_workspace, original_window, cx| {
                            original_window
                                .dispatch_action(zed_actions::OpenKeymap.boxed_clone(), cx);
                            original_window.activate_window();
                        })
                        .ok();
                    window.remove_window();
                }),
                files: USER,
            }),
        ]
    }

    fn base_keymap_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("基础键位映射"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "基础键位映射",
                description: "要使用的基础键位绑定集的名称。",
                field: Box::new(SettingField {
                    json_path: Some("base_keymap"),
                    pick: |settings_content| settings_content.base_keymap.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.base_keymap = value;
                    },
                }),
                metadata: Some(Box::new(SettingsFieldMetadata {
                    should_do_titlecase: Some(false),
                    ..Default::default()
                })),
                files: USER,
            }),
        ]
    }

    fn modal_editing_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("模态编辑"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Vim 模式",
                description: "启用 Vim 模式和键位绑定。",
                field: Box::new(SettingField {
                    json_path: Some("vim_mode"),
                    pick: |settings_content| settings_content.vim_mode.as_ref(),
                    write: write_vim_mode,
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Helix 模式",
                description: "启用 Helix 模式和键位绑定。",
                field: Box::new(SettingField {
                    json_path: Some("helix_mode"),
                    pick: |settings_content| settings_content.helix_mode.as_ref(),
                    write: write_helix_mode,
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    let items: Box<[SettingsPageItem]> = concat_sections!(
        keybindings_section(),
        base_keymap_section(),
        modal_editing_section(),
    );

    SettingsPage {
        title: "键位映射",
        items,
    }
}

fn editor_page() -> SettingsPage {
    fn auto_save_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("自动保存"),
            SettingsPageItem::DynamicItem(DynamicItem {
                discriminant: SettingItem {
                    files: USER,
                    title: "自动保存模式",
                    description: "何时自动保存缓冲区更改。",
                    field: Box::new(SettingField {
                        json_path: Some("autosave$"),
                        pick: |settings_content| {
                            Some(
                                &dynamic_variants::<settings::AutosaveSetting>()[settings_content
                                    .workspace
                                    .autosave
                                    .as_ref()?
                                    .discriminant()
                                    as usize],
                            )
                        },
                        write: |settings_content, value, _| {
                            let Some(value) = value else {
                                settings_content.workspace.autosave = None;
                                return;
                            };
                            let settings_value = settings_content
                                .workspace
                                .autosave
                                .get_or_insert_with(|| settings::AutosaveSetting::Off);
                            *settings_value = match value {
                                settings::AutosaveSettingDiscriminants::Off => {
                                    settings::AutosaveSetting::Off
                                }
                                settings::AutosaveSettingDiscriminants::AfterDelay => {
                                    let milliseconds = match settings_value {
                                        settings::AutosaveSetting::AfterDelay { milliseconds } => {
                                            *milliseconds
                                        }
                                        _ => settings::DelayMs(1000),
                                    };
                                    settings::AutosaveSetting::AfterDelay { milliseconds }
                                }
                                settings::AutosaveSettingDiscriminants::OnFocusChange => {
                                    settings::AutosaveSetting::OnFocusChange
                                }
                                settings::AutosaveSettingDiscriminants::OnWindowChange => {
                                    settings::AutosaveSetting::OnWindowChange
                                }
                            };
                        },
                    }),
                    metadata: None,
                },
                pick_discriminant: |settings_content| {
                    Some(settings_content.workspace.autosave.as_ref()?.discriminant() as usize)
                },
                fields: dynamic_variants::<settings::AutosaveSetting>()
                    .into_iter()
                    .map(|variant| match variant {
                        settings::AutosaveSettingDiscriminants::Off => vec![],
                        settings::AutosaveSettingDiscriminants::AfterDelay => vec![SettingItem {
                            files: USER,
                            title: "延迟(毫秒)",
                            description: "在非活动时段后保存(以毫秒为单位)。",
                            field: Box::new(SettingField {
                                json_path: Some("autosave.after_delay.milliseconds"),
                                pick: |settings_content| match settings_content
                                    .workspace
                                    .autosave
                                    .as_ref()
                                {
                                    Some(settings::AutosaveSetting::AfterDelay {
                                        milliseconds,
                                    }) => Some(milliseconds),
                                    _ => None,
                                },
                                write: |settings_content, value, _| {
                                    let Some(value) = value else {
                                        settings_content.workspace.autosave = None;
                                        return;
                                    };
                                    match settings_content.workspace.autosave.as_mut() {
                                        Some(settings::AutosaveSetting::AfterDelay {
                                            milliseconds,
                                        }) => *milliseconds = value,
                                        _ => return,
                                    }
                                },
                            }),
                            metadata: None,
                        }],
                        settings::AutosaveSettingDiscriminants::OnFocusChange => vec![],
                        settings::AutosaveSettingDiscriminants::OnWindowChange => vec![],
                    })
                    .collect(),
            }),
        ]
    }

    fn which_key_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("Which-key 菜单"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示 Which-key 菜单",
                description: "在多键位绑定待定期间显示具有匹配绑定的 which-key 菜单。",
                field: Box::new(SettingField {
                    json_path: Some("which_key.enabled"),
                    pick: |settings_content| {
                        settings_content
                            .which_key
                            .as_ref()
                            .and_then(|settings| settings.enabled.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content.which_key.get_or_insert_default().enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "菜单延迟",
                description: "Which-key 菜单出现前的延迟(毫秒)。",
                field: Box::new(SettingField {
                    json_path: Some("which_key.delay_ms"),
                    pick: |settings_content| {
                        settings_content
                            .which_key
                            .as_ref()
                            .and_then(|settings| settings.delay_ms.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content.which_key.get_or_insert_default().delay_ms = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn multibuffer_section() -> [SettingsPageItem; 7] {
        [
            SettingsPageItem::SectionHeader("多重缓冲区"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "在多重缓冲区中双击",
                description: "当在多重缓冲区的某些摘录中双击时的操作。",
                field: Box::new(SettingField {
                    json_path: Some("double_click_in_multibuffer"),
                    pick: |settings_content| {
                        settings_content.editor.double_click_in_multibuffer.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.double_click_in_multibuffer = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "展开摘录行数",
                description: "默认情况下展开多重缓冲区摘录的行数。",
                field: Box::new(SettingField {
                    json_path: Some("expand_excerpt_lines"),
                    pick: |settings_content| settings_content.editor.expand_excerpt_lines.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.expand_excerpt_lines = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "摘录上下文行数",
                description: "默认情况下在多重缓冲区摘录中提供的上下文行数。",
                field: Box::new(SettingField {
                    json_path: Some("excerpt_context_lines"),
                    pick: |settings_content| settings_content.editor.excerpt_context_lines.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.excerpt_context_lines = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "展开大纲深度",
                description: "在当前文件中展开大纲项的默认深度。",
                field: Box::new(SettingField {
                    json_path: Some("outline_panel.expand_outlines_with_depth"),
                    pick: |settings_content| {
                        settings_content
                            .outline_panel
                            .as_ref()
                            .and_then(|outline_panel| {
                                outline_panel.expand_outlines_with_depth.as_ref()
                            })
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .expand_outlines_with_depth = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "差异视图样式",
                description: "如何在编辑器中显示差异。",
                field: Box::new(SettingField {
                    json_path: Some("diff_view_style"),
                    pick: |settings_content| settings_content.editor.diff_view_style.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.diff_view_style = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "最小分栏差异宽度",
                description: "使用分栏差异视图的最小宽度 (以列数计)。当编辑器宽度小于此值时, 差异视图会自动切换为统一模式。设置为 0 可禁用此功能。",
                field: Box::new(SettingField {
                    json_path: Some("minimum_split_diff_width"),
                    pick: |settings_content| {
                        settings_content.editor.minimum_split_diff_width.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.minimum_split_diff_width = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn scrolling_section() -> [SettingsPageItem; 9] {
        [
            SettingsPageItem::SectionHeader("滚动"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "滚动超过最后一行",
                description: "编辑器是否允许滚动超过最后一行。",
                field: Box::new(SettingField {
                    json_path: Some("scroll_beyond_last_line"),
                    pick: |settings_content| {
                        settings_content.editor.scroll_beyond_last_line.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.scroll_beyond_last_line = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "垂直滚动边距",
                description: "自动滚动时在光标上方/下方保留的行数。",
                field: Box::new(SettingField {
                    json_path: Some("vertical_scroll_margin"),
                    pick: |settings_content| {
                        settings_content.editor.vertical_scroll_margin.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.vertical_scroll_margin = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "水平滚动边距",
                description: "使用鼠标滚动时在两侧保留的字符数。",
                field: Box::new(SettingField {
                    json_path: Some("horizontal_scroll_margin"),
                    pick: |settings_content| {
                        settings_content.editor.horizontal_scroll_margin.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.horizontal_scroll_margin = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "滚动灵敏度",
                description: "水平和垂直滚动的灵敏度倍数。",
                field: Box::new(SettingField {
                    json_path: Some("scroll_sensitivity"),
                    pick: |settings_content| settings_content.editor.scroll_sensitivity.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.scroll_sensitivity = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "鼠标滚轮缩放",
                description: "是否在按住主修饰键时通过鼠标滚轮缩放编辑器字体大小。",
                field: Box::new(SettingField {
                    json_path: Some("mouse_wheel_zoom"),
                    pick: |settings_content| settings_content.editor.mouse_wheel_zoom.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.mouse_wheel_zoom = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "快速滚动灵敏度",
                description: "水平和垂直快速滚动的灵敏度倍数。",
                field: Box::new(SettingField {
                    json_path: Some("fast_scroll_sensitivity"),
                    pick: |settings_content| {
                        settings_content.editor.fast_scroll_sensitivity.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.fast_scroll_sensitivity = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "点击时自动滚动",
                description: "在可见文本区域边缘附近点击时是否滚动。",
                field: Box::new(SettingField {
                    json_path: Some("autoscroll_on_clicks"),
                    pick: |settings_content| settings_content.editor.autoscroll_on_clicks.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.autoscroll_on_clicks = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "粘性滚动",
                description: "是否将作用域固定在编辑器顶部",
                field: Box::new(SettingField {
                    json_path: Some("sticky_scroll.enabled"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .sticky_scroll
                            .as_ref()
                            .and_then(|sticky_scroll| sticky_scroll.enabled.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .sticky_scroll
                            .get_or_insert_default()
                            .enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn signature_help_section() -> [SettingsPageItem; 4] {
        [
            SettingsPageItem::SectionHeader("签名帮助"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动签名帮助",
                description: "自动显示签名帮助弹窗。",
                field: Box::new(SettingField {
                    json_path: Some("auto_signature_help"),
                    pick: |settings_content| settings_content.editor.auto_signature_help.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.auto_signature_help = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "编辑后显示签名帮助",
                description: "在插入补全或括号对后显示签名帮助弹窗。",
                field: Box::new(SettingField {
                    json_path: Some("show_signature_help_after_edits"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .show_signature_help_after_edits
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.show_signature_help_after_edits = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "代码片段排序",
                description: "确定代码片段相对于其他补全项的排序方式。",
                field: Box::new(SettingField {
                    json_path: Some("snippet_sort_order"),
                    pick: |settings_content| settings_content.editor.snippet_sort_order.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.snippet_sort_order = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn hover_popover_section() -> [SettingsPageItem; 5] {
        [
            SettingsPageItem::SectionHeader("悬停提示框"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用",
                description: "当鼠标在编辑器中的符号上移动时显示信息悬停框。",
                field: Box::new(SettingField {
                    json_path: Some("hover_popover_enabled"),
                    pick: |settings_content| settings_content.editor.hover_popover_enabled.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.hover_popover_enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            // todo(settings ui): add units to this number input
            SettingsPageItem::SettingItem(SettingItem {
                title: "延迟",
                description: "显示信息悬停框前等待的时间(毫秒)。",
                field: Box::new(SettingField {
                    json_path: Some("hover_popover_delay"),
                    pick: |settings_content| settings_content.editor.hover_popover_delay.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.hover_popover_delay = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "置顶",
                description: "鼠标移向悬停提示框时是否保持显示,以便与其内容进行交互。",
                field: Box::new(SettingField {
                    json_path: Some("hover_popover_sticky"),
                    pick: |settings_content| settings_content.editor.hover_popover_sticky.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.hover_popover_sticky = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            // todo(settings ui): add units to this number input
            SettingsPageItem::SettingItem(SettingItem {
                title: "隐藏延迟",
                description: "鼠标移开后,隐藏悬停提示框前等待的毫秒数。",
                field: Box::new(SettingField {
                    json_path: Some("hover_popover_hiding_delay"),
                    pick: |settings_content| {
                        settings_content.editor.hover_popover_hiding_delay.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.hover_popover_hiding_delay = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn drag_and_drop_selection_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("拖放选择"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用",
                description: "启用拖放选择。",
                field: Box::new(SettingField {
                    json_path: Some("drag_and_drop_selection.enabled"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .drag_and_drop_selection
                            .as_ref()
                            .and_then(|drag_and_drop| drag_and_drop.enabled.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .drag_and_drop_selection
                            .get_or_insert_default()
                            .enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "延迟",
                description: "拖放选择开始前的延迟(毫秒)。",
                field: Box::new(SettingField {
                    json_path: Some("drag_and_drop_selection.delay"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .drag_and_drop_selection
                            .as_ref()
                            .and_then(|drag_and_drop| drag_and_drop.delay.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .drag_and_drop_selection
                            .get_or_insert_default()
                            .delay = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn gutter_section() -> [SettingsPageItem; 9] {
        [
            SettingsPageItem::SectionHeader("行号槽"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示行号",
                description: "在行号槽中显示行号。",
                field: Box::new(SettingField {
                    json_path: Some("gutter.line_numbers"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .gutter
                            .as_ref()
                            .and_then(|gutter| gutter.line_numbers.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .gutter
                            .get_or_insert_default()
                            .line_numbers = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "相对行号",
                description: "控制编辑器行号槽中的行号显示。\"disabled\" 显示绝对行号,\"enabled\" 为每个绝对行显示相对行号,而 \"wrapped\" 为每一行(绝对或自动换行)显示相对行号。",
                field: Box::new(SettingField {
                    json_path: Some("relative_line_numbers"),
                    pick: |settings_content| settings_content.editor.relative_line_numbers.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.relative_line_numbers = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示可运行项",
                description: "在行号槽中显示可运行按钮。",
                field: Box::new(SettingField {
                    json_path: Some("gutter.runnables"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .gutter
                            .as_ref()
                            .and_then(|gutter| gutter.runnables.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .gutter
                            .get_or_insert_default()
                            .runnables = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示断点",
                description: "在行号槽中显示断点。",
                field: Box::new(SettingField {
                    json_path: Some("gutter.breakpoints"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .gutter
                            .as_ref()
                            .and_then(|gutter| gutter.breakpoints.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .gutter
                            .get_or_insert_default()
                            .breakpoints = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示书签",
                description: "在行号槽中显示书签。",
                field: Box::new(SettingField {
                    json_path: Some("gutter.bookmarks"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .gutter
                            .as_ref()
                            .and_then(|gutter| gutter.bookmarks.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .gutter
                            .get_or_insert_default()
                            .bookmarks = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示折叠",
                description: "在行号槽中显示代码折叠控件。",
                field: Box::new(SettingField {
                    json_path: Some("gutter.folds"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .gutter
                            .as_ref()
                            .and_then(|gutter| gutter.folds.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.gutter.get_or_insert_default().folds = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "最小行号位数",
                description: "在行号槽中保留空间的最小字符数。",
                field: Box::new(SettingField {
                    json_path: Some("gutter.min_line_number_digits"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .gutter
                            .as_ref()
                            .and_then(|gutter| gutter.min_line_number_digits.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .gutter
                            .get_or_insert_default()
                            .min_line_number_digits = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "内联代码操作",
                description: "在缓冲区行的开头显示代码操作按钮。",
                field: Box::new(SettingField {
                    json_path: Some("inline_code_actions"),
                    pick: |settings_content| settings_content.editor.inline_code_actions.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.inline_code_actions = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn scrollbar_section() -> [SettingsPageItem; 10] {
        [
            SettingsPageItem::SectionHeader("滚动条"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示",
                description: "何时在编辑器中显示滚动条。",
                field: Box::new(SettingField {
                    json_path: Some("scrollbar"),
                    pick: |settings_content| {
                        settings_content.editor.scrollbar.as_ref()?.show.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .scrollbar
                            .get_or_insert_default()
                            .show = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标",
                description: "在滚动条中显示光标位置。",
                field: Box::new(SettingField {
                    json_path: Some("scrollbar.cursors"),
                    pick: |settings_content| {
                        settings_content.editor.scrollbar.as_ref()?.cursors.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .scrollbar
                            .get_or_insert_default()
                            .cursors = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Git 差异",
                description: "在滚动条中显示 Git 差异指示器。",
                field: Box::new(SettingField {
                    json_path: Some("scrollbar.git_diff"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .scrollbar
                            .as_ref()?
                            .git_diff
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .scrollbar
                            .get_or_insert_default()
                            .git_diff = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "搜索结果",
                description: "在滚动条中显示缓冲区搜索结果指示器。",
                field: Box::new(SettingField {
                    json_path: Some("scrollbar.search_results"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .scrollbar
                            .as_ref()?
                            .search_results
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .scrollbar
                            .get_or_insert_default()
                            .search_results = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "选中文本",
                description: "在滚动条中显示选中文本的出现位置。",
                field: Box::new(SettingField {
                    json_path: Some("scrollbar.selected_text"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .scrollbar
                            .as_ref()?
                            .selected_text
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .scrollbar
                            .get_or_insert_default()
                            .selected_text = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "选中符号",
                description: "在滚动条中显示选中符号的出现位置。",
                field: Box::new(SettingField {
                    json_path: Some("scrollbar.selected_symbol"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .scrollbar
                            .as_ref()?
                            .selected_symbol
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .scrollbar
                            .get_or_insert_default()
                            .selected_symbol = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "诊断",
                description: "在滚动条中显示哪些诊断指示器。",
                field: Box::new(SettingField {
                    json_path: Some("scrollbar.diagnostics"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .scrollbar
                            .as_ref()?
                            .diagnostics
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .scrollbar
                            .get_or_insert_default()
                            .diagnostics = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "水平滚动条",
                description: "如果为 false,则强制禁用水平滚动条。",
                field: Box::new(SettingField {
                    json_path: Some("scrollbar.axes.horizontal"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .scrollbar
                            .as_ref()?
                            .axes
                            .as_ref()?
                            .horizontal
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .scrollbar
                            .get_or_insert_default()
                            .axes
                            .get_or_insert_default()
                            .horizontal = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "垂直滚动条",
                description: "如果为 false,则强制禁用垂直滚动条。",
                field: Box::new(SettingField {
                    json_path: Some("scrollbar.axes.vertical"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .scrollbar
                            .as_ref()?
                            .axes
                            .as_ref()?
                            .vertical
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .scrollbar
                            .get_or_insert_default()
                            .axes
                            .get_or_insert_default()
                            .vertical = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn minimap_section() -> [SettingsPageItem; 7] {
        [
            SettingsPageItem::SectionHeader("迷你地图"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示",
                description: "何时在编辑器中显示迷你地图。",
                field: Box::new(SettingField {
                    json_path: Some("minimap.show"),
                    pick: |settings_content| {
                        settings_content.editor.minimap.as_ref()?.show.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.minimap.get_or_insert_default().show = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示位置",
                description: "在编辑器中显示迷你地图的位置。",
                field: Box::new(SettingField {
                    json_path: Some("minimap.display_in"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .minimap
                            .as_ref()?
                            .display_in
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .minimap
                            .get_or_insert_default()
                            .display_in = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "滑块",
                description: "何时显示迷你地图滑块。",
                field: Box::new(SettingField {
                    json_path: Some("minimap.thumb"),
                    pick: |settings_content| {
                        settings_content.editor.minimap.as_ref()?.thumb.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .minimap
                            .get_or_insert_default()
                            .thumb = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Thumb Border",
                description: "迷你地图滚动条滑块的边框样式。",
                field: Box::new(SettingField {
                    json_path: Some("minimap.thumb_border"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .minimap
                            .as_ref()?
                            .thumb_border
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .minimap
                            .get_or_insert_default()
                            .thumb_border = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "当前行高亮",
                description: "如何在迷你地图中高亮当前行。",
                field: Box::new(SettingField {
                    json_path: Some("minimap.current_line_highlight"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .minimap
                            .as_ref()
                            .and_then(|minimap| minimap.current_line_highlight.as_ref())
                            .or(settings_content.editor.current_line_highlight.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .minimap
                            .get_or_insert_default()
                            .current_line_highlight = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "最大宽度列数",
                description: "迷你地图中显示的最大列数。",
                field: Box::new(SettingField {
                    json_path: Some("minimap.max_width_columns"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .minimap
                            .as_ref()?
                            .max_width_columns
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .minimap
                            .get_or_insert_default()
                            .max_width_columns = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn toolbar_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("工具栏"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "面包屑导航",
                description: "显示面包屑导航。",
                field: Box::new(SettingField {
                    json_path: Some("toolbar.breadcrumbs"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .toolbar
                            .as_ref()?
                            .breadcrumbs
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .toolbar
                            .get_or_insert_default()
                            .breadcrumbs = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "快速操作",
                description: "显示快速操作按钮(例如:搜索、选择、编辑器控制等)。",
                field: Box::new(SettingField {
                    json_path: Some("toolbar.quick_actions"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .toolbar
                            .as_ref()?
                            .quick_actions
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .toolbar
                            .get_or_insert_default()
                            .quick_actions = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "选择菜单",
                description: "在编辑器工具栏中显示选择菜单。",
                field: Box::new(SettingField {
                    json_path: Some("toolbar.selections_menu"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .toolbar
                            .as_ref()?
                            .selections_menu
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .toolbar
                            .get_or_insert_default()
                            .selections_menu = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Agent 审查",
                description: "在编辑器工具栏中显示 Agent 审查按钮。",
                field: Box::new(SettingField {
                    json_path: Some("toolbar.agent_review"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .toolbar
                            .as_ref()?
                            .agent_review
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .toolbar
                            .get_or_insert_default()
                            .agent_review = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "代码操作",
                description: "在编辑器工具栏中显示代码操作按钮。",
                field: Box::new(SettingField {
                    json_path: Some("toolbar.code_actions"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .toolbar
                            .as_ref()?
                            .code_actions
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .toolbar
                            .get_or_insert_default()
                            .code_actions = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn vim_settings_section() -> [SettingsPageItem; 14] {
        [
            SettingsPageItem::SectionHeader("Vim"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "默认模式",
                description: "Vim 启动时的默认模式。",
                field: Box::new(SettingField {
                    json_path: Some("vim.default_mode"),
                    pick: |settings_content| settings_content.vim.as_ref()?.default_mode.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.vim.get_or_insert_default().default_mode = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "切换相对行号",
                description: "在 Vim 模式下切换相对行号。",
                field: Box::new(SettingField {
                    json_path: Some("vim.toggle_relative_line_numbers"),
                    pick: |settings_content| {
                        settings_content
                            .vim
                            .as_ref()?
                            .toggle_relative_line_numbers
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .toggle_relative_line_numbers = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用系统剪贴板",
                description: "控制在 Vim 模式下何时使用系统剪贴板。",
                field: Box::new(SettingField {
                    json_path: Some("vim.use_system_clipboard"),
                    pick: |settings_content| {
                        settings_content.vim.as_ref()?.use_system_clipboard.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .use_system_clipboard = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用智能大小写查找",
                description: "在 Vim 模式下启用智能大小写搜索。",
                field: Box::new(SettingField {
                    json_path: Some("vim.use_smartcase_find"),
                    pick: |settings_content| {
                        settings_content.vim.as_ref()?.use_smartcase_find.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .use_smartcase_find = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "全局替换默认值",
                description: "启用后,:substitute 命令默认替换一行中的所有匹配项。'g' 标志随后会切换此行为。",
                field: Box::new(SettingField {
                    json_path: Some("vim.gdefault"),
                    pick: |settings_content| settings_content.vim.as_ref()?.gdefault.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.vim.get_or_insert_default().gdefault = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "复制高亮持续时间",
                description: "在 Vim 模式下高亮显示复制文本的持续时间(毫秒)。",
                field: Box::new(SettingField {
                    json_path: Some("vim.highlight_on_yank_duration"),
                    pick: |settings_content| {
                        settings_content
                            .vim
                            .as_ref()?
                            .highlight_on_yank_duration
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .highlight_on_yank_duration = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "正则表达式搜索",
                description: "在 Vim 搜索中默认使用正则表达式搜索。",
                field: Box::new(SettingField {
                    json_path: Some("vim.use_regex_search"),
                    pick: |settings_content| {
                        settings_content.vim.as_ref()?.use_regex_search.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .use_regex_search = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "在普通模式下显示编辑预测",
                description: "是否在普通模式下显示编辑预测。默认情况下,编辑预测仅在插入和替换模式下显示。",
                field: Box::new(SettingField {
                    json_path: Some("vim.show_edit_predictions_in_normal_mode"),
                    pick: |settings_content| {
                        settings_content
                            .vim
                            .as_ref()?
                            .show_edit_predictions_in_normal_mode
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .show_edit_predictions_in_normal_mode = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标形状 - 普通模式",
                description: "普通模式下的光标形状。",
                field: Box::new(SettingField {
                    json_path: Some("vim.cursor_shape.normal"),
                    pick: |settings_content| {
                        settings_content
                            .vim
                            .as_ref()?
                            .cursor_shape
                            .as_ref()?
                            .normal
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .cursor_shape
                            .get_or_insert_default()
                            .normal = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标形状 - 插入模式",
                description: "插入模式下的光标形状。Inherit 表示使用编辑器的光标形状。",
                field: Box::new(SettingField {
                    json_path: Some("vim.cursor_shape.insert"),
                    pick: |settings_content| {
                        settings_content
                            .vim
                            .as_ref()?
                            .cursor_shape
                            .as_ref()?
                            .insert
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .cursor_shape
                            .get_or_insert_default()
                            .insert = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标形状 - 替换模式",
                description: "替换模式下的光标形状。",
                field: Box::new(SettingField {
                    json_path: Some("vim.cursor_shape.replace"),
                    pick: |settings_content| {
                        settings_content
                            .vim
                            .as_ref()?
                            .cursor_shape
                            .as_ref()?
                            .replace
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .cursor_shape
                            .get_or_insert_default()
                            .replace = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标形状 - 可视模式",
                description: "可视模式下的光标形状。",
                field: Box::new(SettingField {
                    json_path: Some("vim.cursor_shape.visual"),
                    pick: |settings_content| {
                        settings_content
                            .vim
                            .as_ref()?
                            .cursor_shape
                            .as_ref()?
                            .visual
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .vim
                            .get_or_insert_default()
                            .cursor_shape
                            .get_or_insert_default()
                            .visual = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自定义二合字符",
                description: "Vim 模式的自定义二合字符映射。",
                field: Box::new(
                    SettingField {
                        json_path: Some("vim.custom_digraphs"),
                        pick: |settings_content| {
                            settings_content.vim.as_ref()?.custom_digraphs.as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content.vim.get_or_insert_default().custom_digraphs = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER,
            }),
        ]
    }

    let items = concat_sections!(
        auto_save_section(),
        which_key_section(),
        multibuffer_section(),
        scrolling_section(),
        signature_help_section(),
        hover_popover_section(),
        drag_and_drop_selection_section(),
        gutter_section(),
        scrollbar_section(),
        minimap_section(),
        toolbar_section(),
        vim_settings_section(),
        language_settings_data(),
    );

    SettingsPage {
        title: "编辑器",
        items: items,
    }
}

fn languages_and_tools_page(cx: &App) -> SettingsPage {
    fn file_types_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("文件类型"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件类型关联",
                description: "从语言到应被视为该语言的文件和文件扩展名的映射。",
                field: Box::new(
                    SettingField {
                        json_path: Some("file_type_associations"),
                        pick: |settings_content| {
                            settings_content.project.all_languages.file_types.as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content.project.all_languages.file_types = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn diagnostics_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("诊断"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "最高严重性",
                description: "用于过滤编辑器中显示的诊断的级别。",
                field: Box::new(SettingField {
                    json_path: Some("diagnostics_max_severity"),
                    pick: |settings_content| {
                        settings_content.editor.diagnostics_max_severity.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.diagnostics_max_severity = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "包含警告",
                description: "默认情况下是否显示警告。",
                field: Box::new(SettingField {
                    json_path: Some("diagnostics.include_warnings"),
                    pick: |settings_content| {
                        settings_content
                            .diagnostics
                            .as_ref()?
                            .include_warnings
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .diagnostics
                            .get_or_insert_default()
                            .include_warnings = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn inline_diagnostics_section() -> [SettingsPageItem; 5] {
        [
            SettingsPageItem::SectionHeader("内联诊断"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用",
                description: "是否内联显示诊断信息。",
                field: Box::new(SettingField {
                    json_path: Some("diagnostics.inline.enabled"),
                    pick: |settings_content| {
                        settings_content
                            .diagnostics
                            .as_ref()?
                            .inline
                            .as_ref()?
                            .enabled
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .diagnostics
                            .get_or_insert_default()
                            .inline
                            .get_or_insert_default()
                            .enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "更新防抖",
                description: "在最后一次诊断更新后显示内联诊断的延迟(毫秒)。",
                field: Box::new(SettingField {
                    json_path: Some("diagnostics.inline.update_debounce_ms"),
                    pick: |settings_content| {
                        settings_content
                            .diagnostics
                            .as_ref()?
                            .inline
                            .as_ref()?
                            .update_debounce_ms
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .diagnostics
                            .get_or_insert_default()
                            .inline
                            .get_or_insert_default()
                            .update_debounce_ms = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "内边距",
                description: "源代码行末尾与内联诊断开始之间的内边距量。",
                field: Box::new(SettingField {
                    json_path: Some("diagnostics.inline.padding"),
                    pick: |settings_content| {
                        settings_content
                            .diagnostics
                            .as_ref()?
                            .inline
                            .as_ref()?
                            .padding
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .diagnostics
                            .get_or_insert_default()
                            .inline
                            .get_or_insert_default()
                            .padding = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "最小列",
                description: "显示内联诊断的最小列号。",
                field: Box::new(SettingField {
                    json_path: Some("diagnostics.inline.min_column"),
                    pick: |settings_content| {
                        settings_content
                            .diagnostics
                            .as_ref()?
                            .inline
                            .as_ref()?
                            .min_column
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .diagnostics
                            .get_or_insert_default()
                            .inline
                            .get_or_insert_default()
                            .min_column = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn lsp_pull_diagnostics_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("LSP 拉取诊断"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用",
                description: "是否拉取由语言服务器支持的诊断信息。",
                field: Box::new(SettingField {
                    json_path: Some("diagnostics.lsp_pull_diagnostics.enabled"),
                    pick: |settings_content| {
                        settings_content
                            .diagnostics
                            .as_ref()?
                            .lsp_pull_diagnostics
                            .as_ref()?
                            .enabled
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .diagnostics
                            .get_or_insert_default()
                            .lsp_pull_diagnostics
                            .get_or_insert_default()
                            .enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            // todo(settings_ui): Needs unit
            SettingsPageItem::SettingItem(SettingItem {
                title: "防抖",
                description: "从语言服务器拉取诊断之前等待的最短时间。",
                field: Box::new(SettingField {
                    json_path: Some("diagnostics.lsp_pull_diagnostics.debounce_ms"),
                    pick: |settings_content| {
                        settings_content
                            .diagnostics
                            .as_ref()?
                            .lsp_pull_diagnostics
                            .as_ref()?
                            .debounce_ms
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .diagnostics
                            .get_or_insert_default()
                            .lsp_pull_diagnostics
                            .get_or_insert_default()
                            .debounce_ms = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn lsp_highlights_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("LSP 高亮"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "防抖",
                description: "从语言服务器查询高亮之前的防抖延迟。",
                field: Box::new(SettingField {
                    json_path: Some("lsp_highlight_debounce"),
                    pick: |settings_content| {
                        settings_content.editor.lsp_highlight_debounce.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.lsp_highlight_debounce = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn languages_list_section(cx: &App) -> Box<[SettingsPageItem]> {
        // todo(settings_ui): Refresh on extension (un)/installed
        // Note that `crates/json_schema_store` solves the same problem, there is probably a way to unify the two
        std::iter::once(SettingsPageItem::SectionHeader("语言"))
            .chain(all_language_names(cx).into_iter().map(|language_name| {
                let link = format!("languages.{language_name}");
                SettingsPageItem::SubPageLink(SubPageLink {
                    title: language_name,
                    r#type: crate::SubPageType::Language,
                    description: None,
                    json_path: Some(link.leak()),
                    in_json: true,
                    files: USER | PROJECT,
                    render: |this, scroll_handle, window, cx| {
                        let items: Box<[SettingsPageItem]> = concat_sections!(
                            language_settings_data(),
                            non_editor_language_settings_data(),
                            edit_prediction_language_settings_section()
                        );
                        this.render_sub_page_items(
                            items.iter().enumerate(),
                            scroll_handle,
                            window,
                            cx,
                        )
                        .into_any_element()
                    },
                })
            }))
            .collect()
    }

    SettingsPage {
        title: "语言和工具",
        items: {
            concat_sections!(
                non_editor_language_settings_data(),
                file_types_section(),
                diagnostics_section(),
                inline_diagnostics_section(),
                lsp_pull_diagnostics_section(),
                lsp_highlights_section(),
                languages_list_section(cx),
            )
        },
    }
}

fn search_and_files_page() -> SettingsPage {
    fn search_section() -> [SettingsPageItem; 9] {
        [
            SettingsPageItem::SectionHeader("搜索"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "全词匹配",
                description: "默认搜索全词。",
                field: Box::new(SettingField {
                    json_path: Some("search.whole_word"),
                    pick: |settings_content| {
                        settings_content.editor.search.as_ref()?.whole_word.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .search
                            .get_or_insert_default()
                            .whole_word = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "区分大小写",
                description: "默认区分大小写搜索。",
                field: Box::new(SettingField {
                    json_path: Some("search.case_sensitive"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .search
                            .as_ref()?
                            .case_sensitive
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .search
                            .get_or_insert_default()
                            .case_sensitive = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用智能大小写搜索",
                description: "是否根据搜索查询自动启用区分大小写搜索。",
                field: Box::new(SettingField {
                    json_path: Some("use_smartcase_search"),
                    pick: |settings_content| settings_content.editor.use_smartcase_search.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.use_smartcase_search = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "包含已忽略",
                description: "默认在搜索结果中包含已忽略的文件。",
                field: Box::new(SettingField {
                    json_path: Some("search.include_ignored"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .search
                            .as_ref()?
                            .include_ignored
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .search
                            .get_or_insert_default()
                            .include_ignored = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "正则表达式",
                description: "默认使用正则表达式搜索。",
                field: Box::new(SettingField {
                    json_path: Some("search.regex"),
                    pick: |settings_content| {
                        settings_content.editor.search.as_ref()?.regex.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.search.get_or_insert_default().regex = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "搜索循环",
                description: "编辑器搜索结果是否循环。",
                field: Box::new(SettingField {
                    json_path: Some("search_wrap"),
                    pick: |settings_content| settings_content.editor.search_wrap.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.search_wrap = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "匹配居中",
                description: "是否在编辑器中将当前匹配项居中",
                field: Box::new(SettingField {
                    json_path: Some("editor.search.center_on_match"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .search
                            .as_ref()
                            .and_then(|search| search.center_on_match.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .search
                            .get_or_insert_default()
                            .center_on_match = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "基于光标填充搜索",
                description: "何时根据光标下的文本填充新搜索的查询。",
                field: Box::new(SettingField {
                    json_path: Some("seed_search_query_from_cursor"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .seed_search_query_from_cursor
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.seed_search_query_from_cursor = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn file_finder_section() -> [SettingsPageItem; 5] {
        [
            SettingsPageItem::SectionHeader("文件查找器"),
            // todo: null by default
            SettingsPageItem::SettingItem(SettingItem {
                title: "搜索中包含已忽略",
                description: "搜索时使用 git 忽略的文件。",
                field: Box::new(SettingField {
                    json_path: Some("file_finder.include_ignored"),
                    pick: |settings_content| {
                        settings_content
                            .file_finder
                            .as_ref()?
                            .include_ignored
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .file_finder
                            .get_or_insert_default()
                            .include_ignored = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件图标",
                description: "在文件查找器中显示文件图标。",
                field: Box::new(SettingField {
                    json_path: Some("file_finder.file_icons"),
                    pick: |settings_content| {
                        settings_content.file_finder.as_ref()?.file_icons.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .file_finder
                            .get_or_insert_default()
                            .file_icons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "模态框最大宽度",
                description: "确定文件查找器相对于可用窗口宽度可以占据多少空间。",
                field: Box::new(SettingField {
                    json_path: Some("file_finder.modal_max_width"),
                    pick: |settings_content| {
                        settings_content
                            .file_finder
                            .as_ref()?
                            .modal_max_width
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .file_finder
                            .get_or_insert_default()
                            .modal_max_width = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "搜索时跳过活动文件聚焦",
                description: "文件查找器是否应在搜索结果中跳过活动文件的聚焦。",
                field: Box::new(SettingField {
                    json_path: Some("file_finder.skip_focus_for_active_in_search"),
                    pick: |settings_content| {
                        settings_content
                            .file_finder
                            .as_ref()?
                            .skip_focus_for_active_in_search
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .file_finder
                            .get_or_insert_default()
                            .skip_focus_for_active_in_search = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn file_scan_section() -> [SettingsPageItem; 5] {
        [
            SettingsPageItem::SectionHeader("文件扫描"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件扫描排除项",
                description: "Files or globs of files that will be excluded by VibeDev entirely. They will be skipped during file scans, file searches, and not be displayed in the project file tree. Takes precedence over \"File Scan Inclusions\"",
                field: Box::new(
                    SettingField {
                        json_path: Some("file_scan_exclusions"),
                        pick: |settings_content| {
                            settings_content
                                .project
                                .worktree
                                .file_scan_exclusions
                                .as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content.project.worktree.file_scan_exclusions = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件扫描包含项",
                description: "Files or globs of files that will be included by VibeDev, even when ignored by git. This is useful for files that are not tracked by git, but are still important to your project. Note that globs that are overly broad can slow down VibeDev's file scanning. \"File Scan Exclusions\" takes precedence over these inclusions",
                field: Box::new(
                    SettingField {
                        json_path: Some("file_scan_inclusions"),
                        pick: |settings_content| {
                            settings_content
                                .project
                                .worktree
                                .file_scan_inclusions
                                .as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content.project.worktree.file_scan_inclusions = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "恢复文件状态",
                description: "重新打开时恢复之前的文件状态。",
                field: Box::new(SettingField {
                    json_path: Some("restore_on_file_reopen"),
                    pick: |settings_content| {
                        settings_content.workspace.restore_on_file_reopen.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.restore_on_file_reopen = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件删除时关闭",
                description: "自动关闭已删除的文件。",
                field: Box::new(SettingField {
                    json_path: Some("close_on_file_delete"),
                    pick: |settings_content| {
                        settings_content.workspace.close_on_file_delete.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.close_on_file_delete = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    SettingsPage {
        title: "搜索与文件",
        items: concat_sections![search_section(), file_finder_section(), file_scan_section()],
    }
}

fn window_and_layout_page() -> SettingsPage {
    fn status_bar_section() -> [SettingsPageItem; 11] {
        [
            SettingsPageItem::SectionHeader("状态栏"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "项目面板按钮",
                description: "在状态栏中显示项目面板按钮。",
                field: Box::new(SettingField {
                    json_path: Some("project_panel.button"),
                    pick: |settings_content| {
                        settings_content.project_panel.as_ref()?.button.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "当前语言按钮",
                description: "在状态栏中显示当前语言按钮。",
                field: Box::new(SettingField {
                    json_path: Some("status_bar.active_language_button"),
                    pick: |settings_content| {
                        settings_content
                            .status_bar
                            .as_ref()?
                            .active_language_button
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .status_bar
                            .get_or_insert_default()
                            .active_language_button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "当前编码按钮",
                description: "控制何时在状态栏中显示当前编码。",
                field: Box::new(SettingField {
                    json_path: Some("status_bar.active_encoding_button"),
                    pick: |settings_content| {
                        settings_content
                            .status_bar
                            .as_ref()?
                            .active_encoding_button
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .status_bar
                            .get_or_insert_default()
                            .active_encoding_button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标位置按钮",
                description: "在状态栏中显示光标位置按钮。",
                field: Box::new(SettingField {
                    json_path: Some("status_bar.cursor_position_button"),
                    pick: |settings_content| {
                        settings_content
                            .status_bar
                            .as_ref()?
                            .cursor_position_button
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .status_bar
                            .get_or_insert_default()
                            .cursor_position_button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "行尾符号按钮",
                description: "在状态栏显示当前行尾符号按钮。",
                field: Box::new(SettingField {
                    json_path: Some("status_bar.line_endings_button"),
                    pick: |settings_content| {
                        settings_content
                            .status_bar
                            .as_ref()?
                            .line_endings_button
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .status_bar
                            .get_or_insert_default()
                            .line_endings_button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "终端按钮",
                description: "在状态栏中显示终端按钮。",
                field: Box::new(SettingField {
                    json_path: Some("terminal.button"),
                    pick: |settings_content| settings_content.terminal.as_ref()?.button.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.terminal.get_or_insert_default().button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "诊断按钮",
                description: "在状态栏中显示项目诊断按钮。",
                field: Box::new(SettingField {
                    json_path: Some("diagnostics.button"),
                    pick: |settings_content| settings_content.diagnostics.as_ref()?.button.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.diagnostics.get_or_insert_default().button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "项目搜索按钮",
                description: "在状态栏中显示项目搜索按钮。",
                field: Box::new(SettingField {
                    json_path: Some("search.button"),
                    pick: |settings_content| {
                        settings_content.editor.search.as_ref()?.button.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .editor
                            .search
                            .get_or_insert_default()
                            .button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "调试器按钮",
                description: "在状态栏中显示调试器按钮。",
                field: Box::new(SettingField {
                    json_path: Some("调试器按钮"),
                    pick: |settings_content| settings_content.debugger.as_ref()?.button.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.debugger.get_or_insert_default().button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "活动文件名",
                description: "在状态栏中显示活动文件的名称。",
                field: Box::new(SettingField {
                    json_path: Some("status_bar.show_active_file"),
                    pick: |settings_content| {
                        settings_content
                            .status_bar
                            .as_ref()?
                            .show_active_file
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .status_bar
                            .get_or_insert_default()
                            .show_active_file = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn title_bar_section() -> [SettingsPageItem; 10] {
        [
            SettingsPageItem::SectionHeader("标题栏"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示分支状态图标",
                description: "在标题栏的分支图标上显示 git 状态指示器。",
                field: Box::new(SettingField {
                    json_path: Some("title_bar.show_branch_status_icon"),
                    pick: |settings_content| {
                        settings_content
                            .title_bar
                            .as_ref()?
                            .show_branch_status_icon
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .title_bar
                            .get_or_insert_default()
                            .show_branch_status_icon = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示分支名称",
                description: "在标题栏中显示分支名称按钮。",
                field: Box::new(SettingField {
                    json_path: Some("显示分支名称"),
                    pick: |settings_content| {
                        settings_content
                            .title_bar
                            .as_ref()?
                            .show_branch_name
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .title_bar
                            .get_or_insert_default()
                            .show_branch_name = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示项目信息",
                description: "在标题栏中显示项目主机和名称。",
                field: Box::new(SettingField {
                    json_path: Some("显示项目信息"),
                    pick: |settings_content| {
                        settings_content
                            .title_bar
                            .as_ref()?
                            .show_project_items
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .title_bar
                            .get_or_insert_default()
                            .show_project_items = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示新功能横幅",
                description: "在标题栏中显示宣布新功能的横幅。",
                field: Box::new(SettingField {
                    json_path: Some("显示新功能横幅"),
                    pick: |settings_content| {
                        settings_content
                            .title_bar
                            .as_ref()?
                            .show_onboarding_banner
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .title_bar
                            .get_or_insert_default()
                            .show_onboarding_banner = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示登录",
                description: "在标题栏中显示登录按钮。",
                field: Box::new(SettingField {
                    json_path: Some("显示登录按钮"),
                    pick: |settings_content| {
                        settings_content.title_bar.as_ref()?.show_sign_in.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .title_bar
                            .get_or_insert_default()
                            .show_sign_in = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示用户菜单",
                description: "在标题栏中显示用户菜单按钮。",
                field: Box::new(SettingField {
                    json_path: Some("显示用户菜单"),
                    pick: |settings_content| {
                        settings_content.title_bar.as_ref()?.show_user_menu.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .title_bar
                            .get_or_insert_default()
                            .show_user_menu = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示用户头像",
                description: "在标题栏中显示用户头像。",
                field: Box::new(SettingField {
                    json_path: Some("显示用户头像"),
                    pick: |settings_content| {
                        settings_content
                            .title_bar
                            .as_ref()?
                            .show_user_picture
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .title_bar
                            .get_or_insert_default()
                            .show_user_picture = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示菜单",
                description: "在标题栏中显示菜单。",
                field: Box::new(SettingField {
                    json_path: Some("显示菜单"),
                    pick: |settings_content| {
                        settings_content.title_bar.as_ref()?.show_menus.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .title_bar
                            .get_or_insert_default()
                            .show_menus = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::DynamicItem(DynamicItem {
                discriminant: SettingItem {
                    files: USER,
                    title: "按钮布局",
                    description:
                        "(仅限 Linux)选择标题栏中窗口控制按钮的排列方式。",
                    field: Box::new(SettingField {
                        json_path: Some("title_bar.button_layout$"),
                        pick: |settings_content| {
                            Some(
                                &dynamic_variants::<settings::WindowButtonLayoutContent>()[settings_content
                                    .title_bar
                                    .as_ref()?
                                    .button_layout
                                    .as_ref()?
                                    .discriminant()
                                    as usize],
                            )
                        },
                        write: |settings_content, value, _| {
                            let Some(value) = value else {
                                settings_content
                                    .title_bar
                                    .get_or_insert_default()
                                    .button_layout = None;
                                return;
                            };

                            let current_custom_layout = settings_content
                                .title_bar
                                .as_ref()
                                .and_then(|title_bar| title_bar.button_layout.as_ref())
                                .and_then(|button_layout| match button_layout {
                                    settings::WindowButtonLayoutContent::Custom(layout) => {
                                        Some(layout.clone())
                                    }
                                    _ => None,
                                });

                            let button_layout = match value {
                                settings::WindowButtonLayoutContentDiscriminants::PlatformDefault => {
                                    settings::WindowButtonLayoutContent::PlatformDefault
                                }
                                settings::WindowButtonLayoutContentDiscriminants::Standard => {
                                    settings::WindowButtonLayoutContent::Standard
                                }
                                settings::WindowButtonLayoutContentDiscriminants::Custom => {
                                    settings::WindowButtonLayoutContent::Custom(
                                        current_custom_layout.unwrap_or_else(|| {
                                            "close:minimize,maximize".to_string()
                                        }),
                                    )
                                }
                            };

                            settings_content
                                .title_bar
                                .get_or_insert_default()
                                .button_layout = Some(button_layout);
                        },
                    }),
                    metadata: None,
                },
                pick_discriminant: |settings_content| {
                    Some(
                        settings_content
                            .title_bar
                            .as_ref()?
                            .button_layout
                            .as_ref()?
                            .discriminant() as usize,
                    )
                },
                fields: dynamic_variants::<settings::WindowButtonLayoutContent>()
                    .into_iter()
                    .map(|variant| match variant {
                        settings::WindowButtonLayoutContentDiscriminants::PlatformDefault => {
                            vec![]
                        }
                        settings::WindowButtonLayoutContentDiscriminants::Standard => vec![],
                        settings::WindowButtonLayoutContentDiscriminants::Custom => vec![
                            SettingItem {
                                files: USER,
                                title: "自定义按钮布局",
                                description:
                                    "GNOME 风格的布局字符串,例如 \"关闭:minimize,maximize\"。",
                                field: Box::new(SettingField {
                                    json_path: Some("title_bar.button_layout"),
                                    pick: |settings_content| match settings_content
                                        .title_bar
                                        .as_ref()?
                                        .button_layout
                                        .as_ref()?
                                    {
                                        settings::WindowButtonLayoutContent::Custom(layout) => {
                                            Some(layout)
                                        }
                                        _ => DEFAULT_EMPTY_STRING,
                                    },
                                    write: |settings_content, value, _| {
                                        settings_content
                                            .title_bar
                                            .get_or_insert_default()
                                            .button_layout = value
                                            .map(settings::WindowButtonLayoutContent::Custom);
                                    },
                                }),
                                metadata: Some(Box::new(SettingsFieldMetadata {
                                    placeholder: Some("close:minimize,maximize"),
                                    ..Default::default()
                                })),
                            },
                        ],
                    })
                    .collect(),
            }),
        ]
    }

    fn tab_bar_section() -> [SettingsPageItem; 9] {
        [
            SettingsPageItem::SectionHeader("标签页栏"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示标签页栏",
                description: "在编辑器中显示标签页栏。",
                field: Box::new(SettingField {
                    json_path: Some("显示标签页栏"),
                    pick: |settings_content| settings_content.tab_bar.as_ref()?.show.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.tab_bar.get_or_insert_default().show = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "在标签页中显示 Git 状态",
                description: "在标签页上显示 Git 文件状态。",
                field: Box::new(SettingField {
                    json_path: Some("在标签页中显示 Git 状态"),
                    pick: |settings_content| settings_content.tabs.as_ref()?.git_status.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.tabs.get_or_insert_default().git_status = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "在标签页中显示文件图标",
                description: "为标签页显示文件图标。",
                field: Box::new(SettingField {
                    json_path: Some("在标签页中显示文件图标"),
                    pick: |settings_content| settings_content.tabs.as_ref()?.file_icons.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.tabs.get_or_insert_default().file_icons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "标签页关闭按钮位置",
                description: "标签页中关闭按钮的位置。",
                field: Box::new(SettingField {
                    json_path: Some("标签页关闭按钮位置"),
                    pick: |settings_content| {
                        settings_content.tabs.as_ref()?.close_position.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.tabs.get_or_insert_default().close_position = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "最大标签页数",
                description: "一个窗格中打开的最大标签页数。不会关闭未保存的标签页。",
                // todo(settings_ui): The default for this value is null and it's use in code
                // is complex, so I'm going to come back to this later
                field: Box::new(
                    SettingField {
                        json_path: Some("max_tabs"),
                        pick: |settings_content| settings_content.workspace.max_tabs.as_ref(),
                        write: |settings_content, value, _| {
                            settings_content.workspace.max_tabs = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示导航历史按钮",
                description: "在标签页栏中显示导航历史按钮。",
                field: Box::new(SettingField {
                    json_path: Some("显示导航历史按钮"),
                    pick: |settings_content| {
                        settings_content
                            .tab_bar
                            .as_ref()?
                            .show_nav_history_buttons
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .tab_bar
                            .get_or_insert_default()
                            .show_nav_history_buttons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示标签页栏按钮",
                description: "显示标签页栏按钮(新建、拆分窗格、缩放)。",
                field: Box::new(SettingField {
                    json_path: Some("显示标签页栏按钮"),
                    pick: |settings_content| {
                        settings_content
                            .tab_bar
                            .as_ref()?
                            .show_tab_bar_buttons
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .tab_bar
                            .get_or_insert_default()
                            .show_tab_bar_buttons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "固定标签页布局",
                description: "在未固定的标签页上方单独一行显示固定的标签页。",
                field: Box::new(SettingField {
                    json_path: Some("固定标签页布局"),
                    pick: |settings_content| {
                        settings_content
                            .tab_bar
                            .as_ref()?
                            .show_pinned_tabs_in_separate_row
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .tab_bar
                            .get_or_insert_default()
                            .show_pinned_tabs_in_separate_row = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn tab_settings_section() -> [SettingsPageItem; 4] {
        [
            SettingsPageItem::SectionHeader("标签页设置"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "关闭时激活",
                description: "关闭当前标签页后的操作。",
                field: Box::new(SettingField {
                    json_path: Some("关闭时激活"),
                    pick: |settings_content| {
                        settings_content.tabs.as_ref()?.activate_on_close.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .tabs
                            .get_or_insert_default()
                            .activate_on_close = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "标签页显示诊断",
                description: "标记哪些包含诊断错误/警告的文件的标签页。",
                field: Box::new(SettingField {
                    json_path: Some("标签页显示诊断"),
                    pick: |settings_content| {
                        settings_content.tabs.as_ref()?.show_diagnostics.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .tabs
                            .get_or_insert_default()
                            .show_diagnostics = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示关闭按钮",
                description: "控制标签页关闭按钮的显示行为。",
                field: Box::new(SettingField {
                    json_path: Some("显示关闭按钮"),
                    pick: |settings_content| {
                        settings_content.tabs.as_ref()?.show_close_button.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .tabs
                            .get_or_insert_default()
                            .show_close_button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn preview_tabs_section() -> [SettingsPageItem; 8] {
        [
            SettingsPageItem::SectionHeader("预览标签页"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用预览标签页",
                description: "将打开的编辑器显示为预览标签页。",
                field: Box::new(SettingField {
                    json_path: Some("启用预览标签页"),
                    pick: |settings_content| {
                        settings_content.preview_tabs.as_ref()?.enabled.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .preview_tabs
                            .get_or_insert_default()
                            .enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "从项目面板启用预览",
                description: "当从项目面板单击打开文件时,是否以预览模式打开标签页。",
                field: Box::new(SettingField {
                    json_path: Some("从项目面板启用预览"),
                    pick: |settings_content| {
                        settings_content
                            .preview_tabs
                            .as_ref()?
                            .enable_preview_from_project_panel
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .preview_tabs
                            .get_or_insert_default()
                            .enable_preview_from_project_panel = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "从文件查找器启用预览",
                description: "当从文件查找器选择文件时,是否以预览模式打开标签页。",
                field: Box::new(SettingField {
                    json_path: Some("从文件查找器启用预览"),
                    pick: |settings_content| {
                        settings_content
                            .preview_tabs
                            .as_ref()?
                            .enable_preview_from_file_finder
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .preview_tabs
                            .get_or_insert_default()
                            .enable_preview_from_file_finder = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "从多重缓冲区启用预览",
                description: "当从多重缓冲区打开文件时,是否以预览模式打开标签页。",
                field: Box::new(SettingField {
                    json_path: Some("从多重缓冲区启用预览"),
                    pick: |settings_content| {
                        settings_content
                            .preview_tabs
                            .as_ref()?
                            .enable_preview_from_multibuffer
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .preview_tabs
                            .get_or_insert_default()
                            .enable_preview_from_multibuffer = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "从代码导航启用多重缓冲区预览",
                description: "当使用代码导航打开多重缓冲区时,是否以预览模式打开标签页。",
                field: Box::new(SettingField {
                    json_path: Some("从代码导航启用多重缓冲区预览"),
                    pick: |settings_content| {
                        settings_content
                            .preview_tabs
                            .as_ref()?
                            .enable_preview_multibuffer_from_code_navigation
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .preview_tabs
                            .get_or_insert_default()
                            .enable_preview_multibuffer_from_code_navigation = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "从代码导航启用文件预览",
                description: "当使用代码导航打开单个文件时,是否以预览模式打开标签页。",
                field: Box::new(SettingField {
                    json_path: Some("从代码导航启用文件预览"),
                    pick: |settings_content| {
                        settings_content
                            .preview_tabs
                            .as_ref()?
                            .enable_preview_file_from_code_navigation
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .preview_tabs
                            .get_or_insert_default()
                            .enable_preview_file_from_code_navigation = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "代码导航时保持预览",
                description: "当使用代码导航离开标签页时,是否保持其预览模式。如果 `enable_preview_file_from_code_navigation` 或 `enable_preview_multibuffer_from_code_navigation` 也为真,则新标签页可能会替换现有的标签页。",
                field: Box::new(SettingField {
                    json_path: Some("代码导航时保持预览"),
                    pick: |settings_content| {
                        settings_content
                            .preview_tabs
                            .as_ref()?
                            .enable_keep_preview_on_code_navigation
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .preview_tabs
                            .get_or_insert_default()
                            .enable_keep_preview_on_code_navigation = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn layout_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("布局"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "底部停靠栏布局",
                description: "底部停靠栏的布局模式。",
                field: Box::new(SettingField {
                    json_path: Some("bottom_dock_layout"),
                    pick: |settings_content| settings_content.workspace.bottom_dock_layout.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.workspace.bottom_dock_layout = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "居中布局左侧内边距",
                description: "居中布局的左侧内边距。",
                field: Box::new(SettingField {
                    json_path: Some("居中布局左侧内边距"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .centered_layout
                            .as_ref()?
                            .left_padding
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .workspace
                            .centered_layout
                            .get_or_insert_default()
                            .left_padding = value;
                    },
                }),
                metadata: None,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "居中布局右侧内边距",
                description: "居中布局的右侧内边距。",
                field: Box::new(SettingField {
                    json_path: Some("居中布局右侧内边距"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .centered_layout
                            .as_ref()?
                            .right_padding
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .workspace
                            .centered_layout
                            .get_or_insert_default()
                            .right_padding = value;
                    },
                }),
                metadata: None,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "焦点跟随鼠标",
                description: "鼠标悬停在窗格上时是否切换焦点。",
                field: Box::new(SettingField {
                    json_path: Some("focus_follows_mouse.enabled"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .focus_follows_mouse
                            .as_ref()
                            .and_then(|s| s.enabled.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .workspace
                            .focus_follows_mouse
                            .get_or_insert_default()
                            .enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "焦点跟随鼠标防抖毫秒数",
                description: "切换焦点前等待的时间。",
                field: Box::new(SettingField {
                    json_path: Some("focus_follows_mouse.debounce_ms"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .focus_follows_mouse
                            .as_ref()
                            .and_then(|s| s.debounce_ms.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .workspace
                            .focus_follows_mouse
                            .get_or_insert_default()
                            .debounce_ms = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn window_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("窗口"),
            // todo(settings_ui): Should we filter by platform.as_ref()?
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用系统窗口标签页",
                description: "(仅限 macOS) 是否允许 Windows 标签页合并。",
                field: Box::new(SettingField {
                    json_path: Some("use_system_window_tabs"),
                    pick: |settings_content| {
                        settings_content.workspace.use_system_window_tabs.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.use_system_window_tabs = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "窗口装饰",
                description: "(Linux only) whether VibeDev or your compositor should draw window decorations.",
                field: Box::new(SettingField {
                    json_path: Some("window_decorations"),
                    pick: |settings_content| settings_content.workspace.window_decorations.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.workspace.window_decorations = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn pane_modifiers_section() -> [SettingsPageItem; 4] {
        [
            SettingsPageItem::SectionHeader("窗格修饰符"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "非活动不透明度",
                description: "非活动面板的不透明度 (0.0 - 1.0)。",
                field: Box::new(SettingField {
                    json_path: Some("非活动不透明度"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .active_pane_modifiers
                            .as_ref()?
                            .inactive_opacity
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .workspace
                            .active_pane_modifiers
                            .get_or_insert_default()
                            .inactive_opacity = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "边框大小",
                description: "活动窗格周围边框的大小。",
                field: Box::new(SettingField {
                    json_path: Some("边框大小"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .active_pane_modifiers
                            .as_ref()?
                            .border_size
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .workspace
                            .active_pane_modifiers
                            .get_or_insert_default()
                            .border_size = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "缩放内边距",
                description: "为缩放后的窗格显示内边距。",
                field: Box::new(SettingField {
                    json_path: Some("zoomed_padding"),
                    pick: |settings_content| settings_content.workspace.zoomed_padding.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.workspace.zoomed_padding = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn pane_split_direction_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("窗格拆分方向"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "垂直拆分方向",
                description: "垂直拆分的方向。",
                field: Box::new(SettingField {
                    json_path: Some("pane_split_direction_vertical"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .pane_split_direction_vertical
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.pane_split_direction_vertical = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "水平拆分方向",
                description: "水平拆分的方向。",
                field: Box::new(SettingField {
                    json_path: Some("pane_split_direction_horizontal"),
                    pick: |settings_content| {
                        settings_content
                            .workspace
                            .pane_split_direction_horizontal
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.workspace.pane_split_direction_horizontal = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    SettingsPage {
        title: "窗口与布局",
        items: concat_sections![
            status_bar_section(),
            title_bar_section(),
            tab_bar_section(),
            tab_settings_section(),
            preview_tabs_section(),
            layout_section(),
            window_section(),
            pane_modifiers_section(),
            pane_split_direction_section(),
        ],
    }
}

fn panels_page() -> SettingsPage {
    fn project_panel_section() -> [SettingsPageItem; 29] {
        [
            SettingsPageItem::SectionHeader("项目面板"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "项目面板停靠位置",
                description: "项目面板的停靠位置。",
                field: Box::new(SettingField {
                    json_path: Some("项目面板停靠位置"),
                    pick: |settings_content| settings_content.project_panel.as_ref()?.dock.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.project_panel.get_or_insert_default().dock = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "项目面板默认宽度",
                description: "项目面板的默认宽度(像素)。",
                field: Box::new(SettingField {
                    json_path: Some("项目面板默认宽度"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .default_width
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .default_width = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "隐藏 .gitignore",
                description: "是否在项目面板中隐藏 gitignore 条目。",
                field: Box::new(SettingField {
                    json_path: Some("隐藏 .gitignore"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .hide_gitignore
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .hide_gitignore = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "条目间距",
                description: "项目面板中工作树条目之间的间距。",
                field: Box::new(SettingField {
                    json_path: Some("条目间距"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .entry_spacing
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .entry_spacing = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件图标",
                description: "在项目面板中显示文件图标。",
                field: Box::new(SettingField {
                    json_path: Some("文件图标"),
                    pick: |settings_content| {
                        settings_content.project_panel.as_ref()?.file_icons.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .file_icons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件夹图标",
                description: "在项目面板中是显示文件夹图标还是 V 形符号。",
                field: Box::new(SettingField {
                    json_path: Some("文件夹图标"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .folder_icons
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .folder_icons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Git 状态",
                description: "在项目面板中显示 Git 状态。",
                field: Box::new(SettingField {
                    json_path: Some("Git 状态"),
                    pick: |settings_content| {
                        settings_content.project_panel.as_ref()?.git_status.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .git_status = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "缩进大小",
                description: "嵌套项目的缩进量。",
                field: Box::new(SettingField {
                    json_path: Some("缩进大小"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .indent_size
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .indent_size = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动显示条目",
                description: "当相应的项目条目变为活动状态时,是否自动在项目面板中显示该条目。",
                field: Box::new(SettingField {
                    json_path: Some("自动显示条目"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .auto_reveal_entries
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .auto_reveal_entries = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启动时打开",
                description: "项目面板是否应在启动时打开。",
                field: Box::new(SettingField {
                    json_path: Some("启动时打开"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .starts_open
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .starts_open = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动折叠目录",
                description: "当目录内仅包含一个子目录时,是否自动折叠目录并显示紧凑文件夹。",
                field: Box::new(SettingField {
                    json_path: Some("自动折叠目录"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .auto_fold_dirs
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .auto_fold_dirs = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件夹名称加粗",
                description: "是否在项目面板中以粗体文本显示文件夹名称。",
                field: Box::new(SettingField {
                    json_path: Some("文件夹名称加粗"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .bold_folder_labels
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .bold_folder_labels = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示滚动条",
                description: "在项目面板中显示滚动条。",
                field: Box::new(SettingField {
                    json_path: Some("显示滚动条"),
                    pick: |settings_content| {
                        show_scrollbar_or_editor(settings_content, |settings_content| {
                            settings_content
                                .project_panel
                                .as_ref()?
                                .scrollbar
                                .as_ref()?
                                .show
                                .as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .scrollbar
                            .get_or_insert_default()
                            .show = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "水平滚动",
                description: "是否允许在项目面板中进行水平滚动。禁用时,视图始终锁定在最左侧位置,长文件名将被截断。",
                field: Box::new(SettingField {
                    json_path: Some("project_panel.scrollbar.horizontal_scroll"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .scrollbar
                            .as_ref()?
                            .horizontal_scroll
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .scrollbar
                            .get_or_insert_default()
                            .horizontal_scroll = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示诊断",
                description: "在项目面板中标记哪些包含诊断错误/警告的文件。",
                field: Box::new(SettingField {
                    json_path: Some("显示诊断"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .show_diagnostics
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .show_diagnostics = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "诊断徽章",
                description: "在项目面板的文件名旁显示错误和警告计数徽章。",
                field: Box::new(SettingField {
                    json_path: Some("project_panel.diagnostic_badges"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .diagnostic_badges
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .diagnostic_badges = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Git 状态指示器",
                description: "在项目面板中的文件名旁显示 Git 状态指示器。",
                field: Box::new(SettingField {
                    json_path: Some("project_panel.git_status_indicator"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .git_status_indicator
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .git_status_indicator = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "粘性滚动",
                description: "是否将父目录固定在项目面板顶部。",
                field: Box::new(SettingField {
                    json_path: Some("粘性滚动"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .sticky_scroll
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .sticky_scroll = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "显示缩进参考线",
                description: "在项目面板中显示缩进参考线。",
                field: Box::new(SettingField {
                    json_path: Some("显示缩进参考线"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .indent_guides
                            .as_ref()?
                            .show
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .indent_guides
                            .get_or_insert_default()
                            .show = value;
                    },
                }),
                metadata: None,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "拖放",
                description: "是否在项目面板中启用拖放操作。",
                field: Box::new(SettingField {
                    json_path: Some("隐藏根目录"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .drag_and_drop
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .drag_and_drop = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "隐藏根目录",
                description: "当窗口中仅打开一个文件夹时,是否隐藏根目录条目。",
                field: Box::new(SettingField {
                    json_path: Some("project_panel.hide_root"),
                    pick: |settings_content| {
                        settings_content.project_panel.as_ref()?.hide_root.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .hide_root = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "隐藏隐藏文件",
                description: "是否在项目面板中隐藏隐藏条目。",
                field: Box::new(SettingField {
                    json_path: Some("隐藏隐藏文件"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .hide_hidden
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .hide_hidden = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "排序模式",
                description: "项目面板中条目的排序顺序。",
                field: Box::new(SettingField {
                    json_path: Some("排序模式"),
                    pick: |settings_content| {
                        settings_content.project_panel.as_ref()?.sort_mode.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .sort_mode = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "排序大小写",
                description: "在项目面板中是否区分大小写对文件和文件夹名称排序。",
                field: Box::new(SettingField {
                    pick: |settings_content| {
                        settings_content.project_panel.as_ref()?.sort_order.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .sort_order = value;
                    },
                    json_path: Some("project_panel.sort_order"),
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "创建时自动打开文件",
                description: "是否在编辑器中自动打开新创建的文件。",
                field: Box::new(SettingField {
                    json_path: Some("创建时"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .auto_open
                            .as_ref()?
                            .on_create
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .auto_open
                            .get_or_insert_default()
                            .on_create = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "粘贴时自动打开文件",
                description: "在粘贴或复制文件后是否自动打开文件。",
                field: Box::new(SettingField {
                    json_path: Some("粘贴时"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .auto_open
                            .as_ref()?
                            .on_paste
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .auto_open
                            .get_or_insert_default()
                            .on_paste = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "拖放时自动打开文件",
                description: "是否自动打开从外部来源拖入的文件。",
                field: Box::new(SettingField {
                    json_path: Some("拖放时"),
                    pick: |settings_content| {
                        settings_content
                            .project_panel
                            .as_ref()?
                            .auto_open
                            .as_ref()?
                            .on_drop
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project_panel
                            .get_or_insert_default()
                            .auto_open
                            .get_or_insert_default()
                            .on_drop = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "隐藏文件",
                description: "用于匹配将被视为“隐藏”并可以从项目面板中隐藏的文件的 Glob 模式。",
                field: Box::new(
                    SettingField {
                        json_path: Some("隐藏文件"),
                        pick: |settings_content| {
                            settings_content.project.worktree.hidden_files.as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content.project.worktree.hidden_files = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn terminal_panel_section() -> [SettingsPageItem; 4] {
        [
            SettingsPageItem::SectionHeader("终端面板"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "终端停靠位置",
                description: "终端面板的停靠位置。",
                field: Box::new(SettingField {
                    json_path: Some("终端停靠位置"),
                    pick: |settings_content| settings_content.terminal.as_ref()?.dock.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.terminal.get_or_insert_default().dock = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "终端面板弹性大小",
                description: "终端面板停靠在左侧或右侧时是否使用弹性(按比例)大小。",
                field: Box::new(SettingField {
                    json_path: Some("terminal.flexible"),
                    pick: |settings_content| settings_content.terminal.as_ref()?.flexible.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.terminal.get_or_insert_default().flexible = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示计数徽章",
                description: "在终端面板图标上显示打开终端数量的徽章。",
                field: Box::new(SettingField {
                    json_path: Some("terminal.show_count_badge"),
                    pick: |settings_content| {
                        settings_content
                            .terminal
                            .as_ref()?
                            .show_count_badge
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .show_count_badge = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn outline_panel_section() -> [SettingsPageItem; 11] {
        [
            SettingsPageItem::SectionHeader("大纲面板"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "大纲面板按钮",
                description: "在状态栏中显示大纲面板按钮。",
                field: Box::new(SettingField {
                    json_path: Some("大纲面板按钮"),
                    pick: |settings_content| {
                        settings_content.outline_panel.as_ref()?.button.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "大纲面板停靠位置",
                description: "大纲面板的停靠位置。",
                field: Box::new(SettingField {
                    json_path: Some("大纲面板停靠位置"),
                    pick: |settings_content| settings_content.outline_panel.as_ref()?.dock.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.outline_panel.get_or_insert_default().dock = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "大纲面板默认宽度",
                description: "大纲面板的默认宽度(像素)。",
                field: Box::new(SettingField {
                    json_path: Some("大纲面板默认宽度"),
                    pick: |settings_content| {
                        settings_content
                            .outline_panel
                            .as_ref()?
                            .default_width
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .default_width = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件图标",
                description: "在大纲面板中显示文件图标。",
                field: Box::new(SettingField {
                    json_path: Some("文件图标"),
                    pick: |settings_content| {
                        settings_content.outline_panel.as_ref()?.file_icons.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .file_icons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件夹图标",
                description: "在大纲面板中是显示文件夹图标还是 V 形符号。",
                field: Box::new(SettingField {
                    json_path: Some("文件夹图标"),
                    pick: |settings_content| {
                        settings_content
                            .outline_panel
                            .as_ref()?
                            .folder_icons
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .folder_icons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Git 状态",
                description: "在大纲面板中显示 Git 状态。",
                field: Box::new(SettingField {
                    json_path: Some("Git 状态"),
                    pick: |settings_content| {
                        settings_content.outline_panel.as_ref()?.git_status.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .git_status = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "缩进大小",
                description: "嵌套项目的缩进量。",
                field: Box::new(SettingField {
                    json_path: Some("缩进大小"),
                    pick: |settings_content| {
                        settings_content
                            .outline_panel
                            .as_ref()?
                            .indent_size
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .indent_size = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动显示条目",
                description: "当相应的大纲条目变为活动状态时是否显示。",
                field: Box::new(SettingField {
                    json_path: Some("自动显示条目"),
                    pick: |settings_content| {
                        settings_content
                            .outline_panel
                            .as_ref()?
                            .auto_reveal_entries
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .auto_reveal_entries = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动折叠目录",
                description: "当目录仅包含一个子目录时是否自动折叠。",
                field: Box::new(SettingField {
                    json_path: Some("自动折叠目录"),
                    pick: |settings_content| {
                        settings_content
                            .outline_panel
                            .as_ref()?
                            .auto_fold_dirs
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .auto_fold_dirs = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                files: USER,
                title: "显示缩进参考线",
                description: "何时在大纲面板中显示缩进参考线。",
                field: Box::new(SettingField {
                    json_path: Some("显示缩进参考线"),
                    pick: |settings_content| {
                        settings_content
                            .outline_panel
                            .as_ref()?
                            .indent_guides
                            .as_ref()?
                            .show
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .outline_panel
                            .get_or_insert_default()
                            .indent_guides
                            .get_or_insert_default()
                            .show = value;
                    },
                }),
                metadata: None,
            }),
        ]
    }

    fn git_panel_section() -> [SettingsPageItem; 15] {
        [
            SettingsPageItem::SectionHeader("Git 面板"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Git 面板按钮",
                description: "在状态栏中显示 Git 面板按钮。",
                field: Box::new(SettingField {
                    json_path: Some("Git 面板按钮"),
                    pick: |settings_content| settings_content.git_panel.as_ref()?.button.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.git_panel.get_or_insert_default().button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Git 面板停靠位置",
                description: "Git 面板的停靠位置。",
                field: Box::new(SettingField {
                    json_path: Some("Git 面板停靠位置"),
                    pick: |settings_content| settings_content.git_panel.as_ref()?.dock.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.git_panel.get_or_insert_default().dock = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Git 面板默认宽度",
                description: "Git 面板的默认宽度(像素)。",
                field: Box::new(SettingField {
                    json_path: Some("Git 面板默认宽度"),
                    pick: |settings_content| {
                        settings_content.git_panel.as_ref()?.default_width.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .default_width = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Git 面板状态样式",
                description: "条目状态的显示方式。",
                field: Box::new(SettingField {
                    json_path: Some("Git 面板状态样式"),
                    pick: |settings_content| {
                        settings_content.git_panel.as_ref()?.status_style.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .status_style = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "后备分支名称",
                description: "当 Git 中未设置 init.defaultbranch 时的默认分支名称。",
                field: Box::new(SettingField {
                    json_path: Some("后备分支名称"),
                    pick: |settings_content| {
                        settings_content
                            .git_panel
                            .as_ref()?
                            .fallback_branch_name
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .fallback_branch_name = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "按路径排序",
                description: "启用以按路径对面板中的条目排序,禁用则按状态排序。",
                field: Box::new(SettingField {
                    json_path: Some("按路径排序"),
                    pick: |settings_content| {
                        settings_content.git_panel.as_ref()?.sort_by_path.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .sort_by_path = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "折叠未跟踪差异",
                description: "是否在差异面板中折叠未跟踪的文件。",
                field: Box::new(SettingField {
                    json_path: Some("折叠未跟踪差异"),
                    pick: |settings_content| {
                        settings_content
                            .git_panel
                            .as_ref()?
                            .collapse_untracked_diff
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .collapse_untracked_diff = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "树状视图",
                description: "启用以在树状视图列表中显示条目,禁用则在平面视图列表中显示。",
                field: Box::new(SettingField {
                    json_path: Some("树状视图"),
                    pick: |settings_content| {
                        settings_content.git_panel.as_ref()?.tree_view.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.git_panel.get_or_insert_default().tree_view = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件图标",
                description: "在 Git 状态图标旁显示文件图标。",
                field: Box::new(SettingField {
                    json_path: Some("git_panel.file_icons"),
                    pick: |settings_content| {
                        settings_content.git_panel.as_ref()?.file_icons.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .file_icons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "文件夹图标",
                description: "是否在 Git 面板中为目录显示文件夹图标或箭头。",
                field: Box::new(SettingField {
                    json_path: Some("git_panel.folder_icons"),
                    pick: |settings_content| {
                        settings_content.git_panel.as_ref()?.folder_icons.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .folder_icons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "差异统计",
                description: "是否在 Git 面板的每个文件旁显示新增/删除的更改计数。",
                field: Box::new(SettingField {
                    json_path: Some("git_panel.diff_stats"),
                    pick: |settings_content| {
                        settings_content.git_panel.as_ref()?.diff_stats.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .diff_stats = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示计数徽章",
                description: "是否在 Git 面板图标上显示未提交更改数量的徽章。",
                field: Box::new(SettingField {
                    json_path: Some("git_panel.show_count_badge"),
                    pick: |settings_content| {
                        settings_content
                            .git_panel
                            .as_ref()?
                            .show_count_badge
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .show_count_badge = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "提交标题最大长度",
                description: "显示警告前提交消息标题的最大长度。设置为 0 可禁用。",
                field: Box::new(SettingField {
                    json_path: Some("git_panel.commit_title_max_length"),
                    pick: |settings_content| {
                        settings_content
                            .git_panel
                            .as_ref()?
                            .commit_title_max_length
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .commit_title_max_length = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "滚动条",
                description: "滚动条的显示方式及时间。",
                field: Box::new(SettingField {
                    json_path: Some("滚动条"),
                    pick: |settings_content| {
                        show_scrollbar_or_editor(settings_content, |settings_content| {
                            settings_content
                                .git_panel
                                .as_ref()?
                                .scrollbar
                                .as_ref()?
                                .show
                                .as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git_panel
                            .get_or_insert_default()
                            .scrollbar
                            .get_or_insert_default()
                            .show = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn debugger_panel_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("调试器面板"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "调试器面板停靠位置",
                description: "调试面板的停靠位置。",
                field: Box::new(SettingField {
                    json_path: Some("调试器面板停靠位置"),
                    pick: |settings_content| settings_content.debugger.as_ref()?.dock.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.debugger.get_or_insert_default().dock = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn collaboration_panel_section() -> [SettingsPageItem; 4] {
        [
            SettingsPageItem::SectionHeader("协作面板"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "协作面板按钮",
                description: "在状态栏中显示协作面板按钮。",
                field: Box::new(SettingField {
                    json_path: Some("协作面板按钮"),
                    pick: |settings_content| {
                        settings_content
                            .collaboration_panel
                            .as_ref()?
                            .button
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .collaboration_panel
                            .get_or_insert_default()
                            .button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "协作面板停靠位置",
                description: "协作面板的停靠位置。",
                field: Box::new(SettingField {
                    json_path: Some("协作面板默认宽度"),
                    pick: |settings_content| {
                        settings_content.collaboration_panel.as_ref()?.dock.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .collaboration_panel
                            .get_or_insert_default()
                            .dock = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "协作面板默认宽度",
                description: "协作面板的默认宽度(像素)。",
                field: Box::new(SettingField {
                    json_path: Some("协作面板默认宽度"),
                    pick: |settings_content| {
                        settings_content
                            .collaboration_panel
                            .as_ref()?
                            .default_width
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .collaboration_panel
                            .get_or_insert_default()
                            .default_width = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn agent_panel_section() -> [SettingsPageItem; 7] {
        [
            SettingsPageItem::SectionHeader("Agent 面板"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "助手面板按钮",
                description: "是否在状态栏中显示助手面板按钮。",
                field: Box::new(SettingField {
                    json_path: Some("助手面板按钮"),
                    pick: |settings_content| settings_content.agent.as_ref()?.button.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.agent.get_or_insert_default().button = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "助手面板停靠位置",
                description: "助手面板的停靠位置。",
                field: Box::new(SettingField {
                    json_path: Some("助手面板停靠位置"),
                    pick: |settings_content| settings_content.agent.as_ref()?.dock.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.agent.get_or_insert_default().dock = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "助手面板弹性大小",
                description: "助手面板停靠在左侧或右侧时是否使用弹性(按比例)大小。",
                field: Box::new(SettingField {
                    json_path: Some("agent.flexible"),
                    pick: |settings_content| settings_content.agent.as_ref()?.flexible.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.agent.get_or_insert_default().flexible = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "助手面板默认宽度",
                description: "助手面板停靠在左侧或右侧时的默认宽度。",
                field: Box::new(SettingField {
                    json_path: Some("助手面板默认宽度"),
                    pick: |settings_content| {
                        settings_content.agent.as_ref()?.default_width.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.agent.get_or_insert_default().default_width = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "助手面板默认高度",
                description: "助手面板停靠在底部时的默认高度。",
                field: Box::new(SettingField {
                    json_path: Some("助手面板默认高度"),
                    pick: |settings_content| {
                        settings_content.agent.as_ref()?.default_height.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .default_height = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::DynamicItem(DynamicItem {
                discriminant: SettingItem {
                    files: USER,
                    title: "限制内容宽度",
                    description: "是否将助手面板内容限制在最大宽度内,当面板更宽时居中显示内容以获得最佳可读性。",
                    field: Box::new(SettingField::<bool> {
                        json_path: Some("agent.limit_content_width"),
                        pick: |settings_content| {
                            settings_content
                                .agent
                                .as_ref()?
                                .limit_content_width
                                .as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content
                                .agent
                                .get_or_insert_default()
                                .limit_content_width = value;
                        },
                    }),
                    metadata: None,
                },
                pick_discriminant: |settings_content| {
                    let enabled = settings_content
                        .agent
                        .as_ref()?
                        .limit_content_width
                        .unwrap_or(true);
                    Some(if enabled { 1 } else { 0 })
                },
                fields: vec![
                    vec![],
                    vec![SettingItem {
                        files: USER,
                        title: "最大内容宽度",
                        description: "最大内容宽度(像素)。当面板宽度大于此值时, 内容将居中显示。",
                        field: Box::new(SettingField {
                            json_path: Some("agent.max_content_width"),
                            pick: |settings_content| {
                                settings_content.agent.as_ref()?.max_content_width.as_ref()
                            },
                            write: |settings_content, value, _| {
                                settings_content
                                    .agent
                                    .get_or_insert_default()
                                    .max_content_width = value;
                            },
                        }),
                        metadata: None,
                    }],
                ],
            }),
        ]
    }

    SettingsPage {
        title: "面板",
        items: concat_sections![
            project_panel_section(),
            terminal_panel_section(),
            outline_panel_section(),
            git_panel_section(),
            debugger_panel_section(),
            collaboration_panel_section(),
            agent_panel_section(),
        ],
    }
}

fn debugger_page() -> SettingsPage {
    fn general_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("常规"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "单步执行粒度",
                description: "确定调试操作的单步执行粒度。",
                field: Box::new(SettingField {
                    json_path: Some("单步执行粒度"),
                    pick: |settings_content| {
                        settings_content
                            .debugger
                            .as_ref()?
                            .stepping_granularity
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .debugger
                            .get_or_insert_default()
                            .stepping_granularity = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "保存断点",
                description: "Whether breakpoints should be reused across VibeDev sessions.",
                field: Box::new(SettingField {
                    json_path: Some("保存断点"),
                    pick: |settings_content| {
                        settings_content
                            .debugger
                            .as_ref()?
                            .save_breakpoints
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .debugger
                            .get_or_insert_default()
                            .save_breakpoints = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "超时",
                description: "连接到 TCP 调试适配器时直到超时错误的时间(毫秒)。",
                field: Box::new(SettingField {
                    json_path: Some("超时"),
                    pick: |settings_content| settings_content.debugger.as_ref()?.timeout.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.debugger.get_or_insert_default().timeout = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "记录 DAP 通信",
                description: "Whether to log messages between active debug adapters and VibeDev.",
                field: Box::new(SettingField {
                    json_path: Some("记录 DAP 通信"),
                    pick: |settings_content| {
                        settings_content
                            .debugger
                            .as_ref()?
                            .log_dap_communications
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .debugger
                            .get_or_insert_default()
                            .log_dap_communications = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "格式化 DAP 日志消息",
                description: "在将 DAP 消息添加到调试适配器记录器时是否格式化它们。",
                field: Box::new(SettingField {
                    json_path: Some("格式化 DAP 日志消息"),
                    pick: |settings_content| {
                        settings_content
                            .debugger
                            .as_ref()?
                            .format_dap_log_messages
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .debugger
                            .get_or_insert_default()
                            .format_dap_log_messages = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    SettingsPage {
        title: "调试器",
        items: concat_sections![general_section()],
    }
}

fn terminal_page() -> SettingsPage {
    fn environment_section() -> [SettingsPageItem; 5] {
        [
                SettingsPageItem::SectionHeader("环境"),
                SettingsPageItem::DynamicItem(DynamicItem {
                    discriminant: SettingItem {
                        files: USER | PROJECT,
                        title: "Shell",
                        description: "打开终端时使用哪个 Shell。",
                        field: Box::new(SettingField {
                            json_path: Some("Shell"),
                            pick: |settings_content| {
                                Some(&dynamic_variants::<settings::Shell>()[
                                    settings_content
                                        .terminal
                                        .as_ref()?
                                        .project
                                        .shell
                                        .as_ref()?
                                        .discriminant() as usize
                                ])
                            },
                            write: |settings_content, value, _| {
                                let Some(value) = value else {
                                    if let Some(terminal) = settings_content.terminal.as_mut() {
                                        terminal.project.shell = None;
                                    }
                                    return;
                                };
                                let settings_value = settings_content
                                    .terminal
                                    .get_or_insert_default()
                                    .project
                                    .shell
                                    .get_or_insert_with(|| settings::Shell::default());
                                let default_shell = if cfg!(target_os = "windows") {
                                    "powershell.exe"
                                } else {
                                    "sh"
                                };
                                *settings_value = match value {
                                    settings::ShellDiscriminants::System => settings::Shell::System,
                                    settings::ShellDiscriminants::Program => {
                                        let program = match settings_value {
                                            settings::Shell::Program(program) => program.clone(),
                                            settings::Shell::WithArguments { program, .. } => program.clone(),
                                            _ => String::from(default_shell),
                                        };
                                        settings::Shell::Program(program)
                                    }
                                    settings::ShellDiscriminants::WithArguments => {
                                        let (program, args, title_override) = match settings_value {
                                            settings::Shell::Program(program) => (program.clone(), vec![], None),
                                            settings::Shell::WithArguments {
                                                program,
                                                args,
                                                title_override,
                                            } => (program.clone(), args.clone(), title_override.clone()),
                                            _ => (String::from(default_shell), vec![], None),
                                        };
                                        settings::Shell::WithArguments {
                                            program,
                                            args,
                                            title_override,
                                        }
                                    }
                                };
                            },
                        }),
                        metadata: None,
                    },
                    pick_discriminant: |settings_content| {
                        Some(
                            settings_content
                                .terminal
                                .as_ref()?
                                .project
                                .shell
                                .as_ref()?
                                .discriminant() as usize,
                        )
                    },
                    fields: dynamic_variants::<settings::Shell>()
                        .into_iter()
                        .map(|variant| match variant {
                            settings::ShellDiscriminants::System => vec![],
                            settings::ShellDiscriminants::Program => vec![SettingItem {
                                files: USER | PROJECT,
                                title: "程序",
                                description: "要使用的 Shell 程序。",
                                field: Box::new(SettingField {
                                    json_path: Some("程序"),
                                    pick: |settings_content| match settings_content.terminal.as_ref()?.project.shell.as_ref()
                                    {
                                        Some(settings::Shell::Program(program)) => Some(program),
                                        _ => None,
                                    },
                                    write: |settings_content, value, _| {
                                        let Some(value) = value else {
                                            return;
                                        };
                                        match settings_content
                                            .terminal
                                            .get_or_insert_default()
                                            .project
                                            .shell
                                            .as_mut()
                                        {
                                            Some(settings::Shell::Program(program)) => *program = value,
                                            _ => return,
                                        }
                                    },
                                }),
                                metadata: None,
                            }],
                            settings::ShellDiscriminants::WithArguments => vec![
                                SettingItem {
                                    files: USER | PROJECT,
                                    title: "程序",
                                    description: "要运行的 Shell 程序。",
                                    field: Box::new(SettingField {
                                        json_path: Some("程序"),
                                        pick: |settings_content| {
                                            match settings_content.terminal.as_ref()?.project.shell.as_ref() {
                                                Some(settings::Shell::WithArguments { program, .. }) => Some(program),
                                                _ => None,
                                            }
                                        },
                                        write: |settings_content, value, _| {
                                            let Some(value) = value else {
                                                return;
                                            };
                                            match settings_content
                                                .terminal
                                                .get_or_insert_default()
                                                .project
                                                .shell
                                                .as_mut()
                                            {
                                                Some(settings::Shell::WithArguments { program, .. }) => {
                                                    *program = value
                                                }
                                                _ => return,
                                            }
                                        },
                                    }),
                                    metadata: None,
                                },
                                SettingItem {
                                    files: USER | PROJECT,
                                    title: "参数",
                                    description: "传递给 Shell 程序的参数。",
                                    field: Box::new(
                                        SettingField {
                                            json_path: Some("参数"),
                                            pick: |settings_content| {
                                                match settings_content.terminal.as_ref()?.project.shell.as_ref() {
                                                    Some(settings::Shell::WithArguments { args, .. }) => Some(args),
                                                    _ => None,
                                                }
                                            },
                                            write: |settings_content, value, _| {
                                                let Some(value) = value else {
                                                    return;
                                                };
                                                match settings_content
                                                    .terminal
                                                    .get_or_insert_default()
                                                    .project
                                                    .shell
                                                    .as_mut()
                                                {
                                                    Some(settings::Shell::WithArguments { args, .. }) => *args = value,
                                                    _ => return,
                                                }
                                            },
                                        }
                                        .unimplemented(),
                                    ),
                                    metadata: None,
                                },
                                SettingItem {
                                    files: USER | PROJECT,
                                    title: "标题覆盖",
                                    description: "用于覆盖终端标签页标题的可选字符串。",
                                    field: Box::new(SettingField {
                                        json_path: Some("标题覆盖"),
                                        pick: |settings_content| {
                                            match settings_content.terminal.as_ref()?.project.shell.as_ref() {
                                                Some(settings::Shell::WithArguments { title_override, .. }) => {
                                                    title_override.as_ref().or(DEFAULT_EMPTY_STRING)
                                                }
                                                _ => None,
                                            }
                                        },
                                        write: |settings_content, value, _| {
                                            match settings_content
                                                .terminal
                                                .get_or_insert_default()
                                                .project
                                                .shell
                                                .as_mut()
                                            {
                                                Some(settings::Shell::WithArguments { title_override, .. }) => {
                                                    *title_override = value.filter(|s| !s.is_empty())
                                                }
                                                _ => return,
                                            }
                                        },
                                    }),
                                    metadata: None,
                                },
                            ],
                        })
                        .collect(),
                }),
                SettingsPageItem::DynamicItem(DynamicItem {
                    discriminant: SettingItem {
                        files: USER | PROJECT,
                        title: "工作目录",
                        description: "启动终端时使用哪个工作目录。",
                        field: Box::new(SettingField {
                            json_path: Some("工作目录"),
                            pick: |settings_content| {
                                Some(&dynamic_variants::<settings::WorkingDirectory>()[
                                    settings_content
                                        .terminal
                                        .as_ref()?
                                        .project
                                        .working_directory
                                        .as_ref()?
                                        .discriminant() as usize
                                ])
                            },
                            write: |settings_content, value, _| {
                                let Some(value) = value else {
                                    if let Some(terminal) = settings_content.terminal.as_mut() {
                                        terminal.project.working_directory = None;
                                    }
                                    return;
                                };
                                let settings_value = settings_content
                                    .terminal
                                    .get_or_insert_default()
                                    .project
                                    .working_directory
                                    .get_or_insert_with(|| settings::WorkingDirectory::CurrentProjectDirectory);
                                *settings_value = match value {
                                    settings::WorkingDirectoryDiscriminants::CurrentFileDirectory => {
                                        settings::WorkingDirectory::CurrentFileDirectory
                                    },
                                    settings::WorkingDirectoryDiscriminants::CurrentProjectDirectory => {
                                        settings::WorkingDirectory::CurrentProjectDirectory
                                    }
                                    settings::WorkingDirectoryDiscriminants::FirstProjectDirectory => {
                                        settings::WorkingDirectory::FirstProjectDirectory
                                    }
                                    settings::WorkingDirectoryDiscriminants::AlwaysHome => {
                                        settings::WorkingDirectory::AlwaysHome
                                    }
                                    settings::WorkingDirectoryDiscriminants::Always => {
                                        let directory = match settings_value {
                                            settings::WorkingDirectory::Always { .. } => return,
                                            _ => String::new(),
                                        };
                                        settings::WorkingDirectory::Always { directory }
                                    }
                                };
                            },
                        }),
                        metadata: None,
                    },
                    pick_discriminant: |settings_content| {
                        Some(
                            settings_content
                                .terminal
                                .as_ref()?
                                .project
                                .working_directory
                                .as_ref()?
                                .discriminant() as usize,
                        )
                    },
                    fields: dynamic_variants::<settings::WorkingDirectory>()
                        .into_iter()
                        .map(|variant| match variant {
                            settings::WorkingDirectoryDiscriminants::CurrentFileDirectory => vec![],
                            settings::WorkingDirectoryDiscriminants::CurrentProjectDirectory => vec![],
                            settings::WorkingDirectoryDiscriminants::FirstProjectDirectory => vec![],
                            settings::WorkingDirectoryDiscriminants::AlwaysHome => vec![],
                            settings::WorkingDirectoryDiscriminants::Always => vec![SettingItem {
                                files: USER | PROJECT,
                                title: "目录",
                                description: "要使用的目录路径(将由 Shell 展开)。",
                                field: Box::new(SettingField {
                                    json_path: Some("目录"),
                                    pick: |settings_content| {
                                        match settings_content.terminal.as_ref()?.project.working_directory.as_ref() {
                                            Some(settings::WorkingDirectory::Always { directory }) => Some(directory),
                                            _ => None,
                                        }
                                    },
                                    write: |settings_content, value, _| {
                                        let value = value.unwrap_or_default();
                                        match settings_content
                                            .terminal
                                            .get_or_insert_default()
                                            .project
                                            .working_directory
                                            .as_mut()
                                        {
                                            Some(settings::WorkingDirectory::Always { directory }) => *directory = value,
                                            _ => return,
                                        }
                                    },
                                }),
                                metadata: None,
                            }],
                        })
                        .collect(),
                }),
                SettingsPageItem::SettingItem(SettingItem {
                    title: "环境变量",
                    description: "添加到终端环境的键值对。",
                    field: Box::new(
                        SettingField {
                            json_path: Some("环境变量"),
                            pick: |settings_content| settings_content.terminal.as_ref()?.project.env.as_ref(),
                            write: |settings_content, value, _| {
                                settings_content.terminal.get_or_insert_default().project.env = value;
                            },
                        }
                        .unimplemented(),
                    ),
                    metadata: None,
                    files: USER | PROJECT,
                }),
                SettingsPageItem::SettingItem(SettingItem {
                    title: "检测虚拟环境",
                    description: "如果在终端的工作目录中找到 Python 虚拟环境,则激活它。",
                    field: Box::new(
                        SettingField {
                            json_path: Some("检测虚拟环境"),
                            pick: |settings_content| settings_content.terminal.as_ref()?.project.detect_venv.as_ref(),
                            write: |settings_content, value, _| {
                                settings_content
                                    .terminal
                                    .get_or_insert_default()
                                    .project
                                    .detect_venv = value;
                            },
                        }
                        .unimplemented(),
                    ),
                    metadata: None,
                    files: USER | PROJECT,
                }),
            ]
    }

    fn font_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("字体"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体大小",
                description: "终端文本的字体大小。如果未设置,默认为缓冲区字体大小。",
                field: Box::new(SettingField {
                    json_path: Some("字体大小"),
                    pick: |settings_content| {
                        settings_content
                            .terminal
                            .as_ref()
                            .and_then(|terminal| terminal.font_size.as_ref())
                            .or(settings_content.theme.buffer_font_size.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content.terminal.get_or_insert_default().font_size = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体系列",
                description: "终端文本的字体族。如果未设置,默认为缓冲区字体族。",
                field: Box::new(SettingField {
                    json_path: Some("字体系列"),
                    pick: |settings_content| {
                        settings_content
                            .terminal
                            .as_ref()
                            .and_then(|terminal| terminal.font_family.as_ref())
                            .or(settings_content.theme.buffer_font_family.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .font_family = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体回退",
                description: "终端文本的字体回退。如果未设置,默认为缓冲区字体回退。",
                field: Box::new(
                    SettingField {
                        json_path: Some("字体回退"),
                        pick: |settings_content| {
                            settings_content
                                .terminal
                                .as_ref()
                                .and_then(|terminal| terminal.font_fallbacks.as_ref())
                                .or(settings_content.theme.buffer_font_fallbacks.as_ref())
                        },
                        write: |settings_content, value, _| {
                            settings_content
                                .terminal
                                .get_or_insert_default()
                                .font_fallbacks = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体粗细",
                description: "终端文本的字体粗细,以 CSS 权重单位 (100-900) 表示。",
                field: Box::new(SettingField {
                    json_path: Some("字体粗细"),
                    pick: |settings_content| {
                        settings_content.terminal.as_ref()?.font_weight.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .font_weight = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "字体特性",
                description: "终端文本的字体特性。",
                field: Box::new(
                    SettingField {
                        json_path: Some("字体特性"),
                        pick: |settings_content| {
                            settings_content
                                .terminal
                                .as_ref()
                                .and_then(|terminal| terminal.font_features.as_ref())
                                .or(settings_content.theme.buffer_font_features.as_ref())
                        },
                        write: |settings_content, value, _| {
                            settings_content
                                .terminal
                                .get_or_insert_default()
                                .font_features = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn display_settings_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("显示设置"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "行高",
                description: "终端文本的行高。",
                field: Box::new(
                    SettingField {
                        json_path: Some("行高"),
                        pick: |settings_content| {
                            settings_content.terminal.as_ref()?.line_height.as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content
                                .terminal
                                .get_or_insert_default()
                                .line_height = value;
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标形状",
                description: "终端的默认光标形状(bar、block、underline 或 hollow)。",
                field: Box::new(SettingField {
                    json_path: Some("光标形状"),
                    pick: |settings_content| {
                        settings_content.terminal.as_ref()?.cursor_shape.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .cursor_shape = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "光标闪烁",
                description: "设置终端中的光标闪烁行为。",
                field: Box::new(SettingField {
                    json_path: Some("光标闪烁"),
                    pick: |settings_content| settings_content.terminal.as_ref()?.blinking.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.terminal.get_or_insert_default().blinking = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "备用滚动",
                description: "是否默认启用备用滚动模式(在 Vim 等应用中将鼠标滚动转换为方向键)。",
                field: Box::new(SettingField {
                    json_path: Some("备用滚动"),
                    pick: |settings_content| {
                        settings_content
                            .terminal
                            .as_ref()?
                            .alternate_scroll
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .alternate_scroll = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "最小对比度",
                description: "前景色和背景色之间的最小 APCA 感知对比度 (0-106)。",
                field: Box::new(SettingField {
                    json_path: Some("最小对比度"),
                    pick: |settings_content| {
                        settings_content
                            .terminal
                            .as_ref()?
                            .minimum_contrast
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .minimum_contrast = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn behavior_settings_section() -> [SettingsPageItem; 5] {
        [
            SettingsPageItem::SectionHeader("行为设置"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Option 键作为 Meta 键",
                description: "Option 键是否作为 Meta 键使用。",
                field: Box::new(SettingField {
                    json_path: Some("Option 键作为 Meta 键"),
                    pick: |settings_content| {
                        settings_content.terminal.as_ref()?.option_as_meta.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .option_as_meta = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "选择时复制",
                description: "在终端中选择文本是否自动复制到系统剪贴板。",
                field: Box::new(SettingField {
                    json_path: Some("选择时复制"),
                    pick: |settings_content| {
                        settings_content.terminal.as_ref()?.copy_on_select.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .copy_on_select = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "复制后保留选择",
                description: "将文本选择复制到剪贴板后是否保留选择状态。",
                field: Box::new(SettingField {
                    json_path: Some("复制后保持选中状态"),
                    pick: |settings_content| {
                        settings_content
                            .terminal
                            .as_ref()?
                            .keep_selection_on_copy
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .keep_selection_on_copy = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "声音提示",
                description: "当打印 BEL 字符(`\\a`、`0x07`)时是否播放声音",
                field: Box::new(SettingField {
                    json_path: Some("terminal.bell"),
                    pick: |settings_content| settings_content.terminal.as_ref()?.bell.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.terminal.get_or_insert_default().bell = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn layout_settings_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("布局设置"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "默认宽度",
                description: "终端停靠在左侧或右侧时的默认宽度(像素)。",
                field: Box::new(SettingField {
                    json_path: Some("默认宽度"),
                    pick: |settings_content| {
                        settings_content.terminal.as_ref()?.default_width.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .default_width = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "默认高度",
                description: "终端停靠在底部时的默认高度(像素)。",
                field: Box::new(SettingField {
                    json_path: Some("默认高度"),
                    pick: |settings_content| {
                        settings_content.terminal.as_ref()?.default_height.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .default_height = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn advanced_settings_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("高级设置"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "最大滚动历史行数",
                description: "滚动历史中保留的最大行数(最大:100,000;0 表示禁用滚动)。",
                field: Box::new(SettingField {
                    json_path: Some("最大滚动历史行数"),
                    pick: |settings_content| {
                        settings_content
                            .terminal
                            .as_ref()?
                            .max_scroll_history_lines
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .max_scroll_history_lines = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "滚动倍率",
                description: "使用鼠标滚轮在终端中滚动的倍率",
                field: Box::new(SettingField {
                    json_path: Some("滚动倍率"),
                    pick: |settings_content| {
                        settings_content
                            .terminal
                            .as_ref()?
                            .scroll_multiplier
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .scroll_multiplier = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn toolbar_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("工具栏"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "面包屑导航",
                description: "在终端窗格内的面包屑导航中显示终端标题。",
                field: Box::new(SettingField {
                    json_path: Some("面包屑导航"),
                    pick: |settings_content| {
                        settings_content
                            .terminal
                            .as_ref()?
                            .toolbar
                            .as_ref()?
                            .breadcrumbs
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .toolbar
                            .get_or_insert_default()
                            .breadcrumbs = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn scrollbar_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("滚动条"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示滚动条",
                description: "何时在终端中显示滚动条。",
                field: Box::new(SettingField {
                    json_path: Some("显示滚动条"),
                    pick: |settings_content| {
                        show_scrollbar_or_editor(settings_content, |settings_content| {
                            settings_content
                                .terminal
                                .as_ref()?
                                .scrollbar
                                .as_ref()?
                                .show
                                .as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .terminal
                            .get_or_insert_default()
                            .scrollbar
                            .get_or_insert_default()
                            .show = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    SettingsPage {
        title: "终端",
        items: concat_sections![
            environment_section(),
            font_section(),
            display_settings_section(),
            behavior_settings_section(),
            layout_settings_section(),
            advanced_settings_section(),
            toolbar_section(),
            scrollbar_section(),
        ],
    }
}

fn version_control_page() -> SettingsPage {
    fn git_integration_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("Git 集成"),
            SettingsPageItem::DynamicItem(DynamicItem {
                discriminant: SettingItem {
                    files: USER,
                    title: "禁用 Git 集成",
                    description: "Disable all Git integration features in VibeDev.",
                    field: Box::new(SettingField::<bool> {
                        json_path: Some("禁用 Git 集成"),
                        pick: |settings_content| {
                            settings_content
                                .git
                                .as_ref()?
                                .enabled
                                .as_ref()?
                                .disable_git
                                .as_ref()
                        },
                        write: |settings_content, value, _| {
                            settings_content
                                .git
                                .get_or_insert_default()
                                .enabled
                                .get_or_insert_default()
                                .disable_git = value;
                        },
                    }),
                    metadata: None,
                },
                pick_discriminant: |settings_content| {
                    let disabled = settings_content
                        .git
                        .as_ref()?
                        .enabled
                        .as_ref()?
                        .disable_git
                        .unwrap_or(false);
                    Some(if disabled { 0 } else { 1 })
                },
                fields: vec![
                    vec![],
                    vec![
                        SettingItem {
                            files: USER,
                            title: "启用 Git 状态",
                            description: "在编辑器中显示 Git 状态信息。",
                            field: Box::new(SettingField::<bool> {
                                json_path: Some("启用 Git 状态"),
                                pick: |settings_content| {
                                    settings_content
                                        .git
                                        .as_ref()?
                                        .enabled
                                        .as_ref()?
                                        .enable_status
                                        .as_ref()
                                },
                                write: |settings_content, value, _| {
                                    settings_content
                                        .git
                                        .get_or_insert_default()
                                        .enabled
                                        .get_or_insert_default()
                                        .enable_status = value;
                                },
                            }),
                            metadata: None,
                        },
                        SettingItem {
                            files: USER,
                            title: "启用 Git 差异",
                            description: "在编辑器中显示 Git 差异信息。",
                            field: Box::new(SettingField::<bool> {
                                json_path: Some("启用 Git 差异"),
                                pick: |settings_content| {
                                    settings_content
                                        .git
                                        .as_ref()?
                                        .enabled
                                        .as_ref()?
                                        .enable_diff
                                        .as_ref()
                                },
                                write: |settings_content, value, _| {
                                    settings_content
                                        .git
                                        .get_or_insert_default()
                                        .enabled
                                        .get_or_insert_default()
                                        .enable_diff = value;
                                },
                            }),
                            metadata: None,
                        },
                    ],
                ],
            }),
        ]
    }

    fn git_gutter_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("Git 行号槽"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "可见性",
                description: "控制是否在编辑器的行号槽中显示 Git 状态。",
                field: Box::new(SettingField {
                    json_path: Some("可见性"),
                    pick: |settings_content| settings_content.git.as_ref()?.git_gutter.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.git.get_or_insert_default().git_gutter = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            // todo(settings_ui): Figure out the right default for this value in default.json
            SettingsPageItem::SettingItem(SettingItem {
                title: "防抖",
                description: "防抖阈值(毫秒),超过此时间后更改会反映在 Git 行号槽中。",
                field: Box::new(SettingField {
                    json_path: Some("防抖"),
                    pick: |settings_content| {
                        settings_content.git.as_ref()?.gutter_debounce.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.git.get_or_insert_default().gutter_debounce = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn inline_git_blame_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("内联 Git 追溯"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用",
                description: "是否在当前聚焦行中内联显示 Git 追溯数据。",
                field: Box::new(SettingField {
                    json_path: Some("启用"),
                    pick: |settings_content| {
                        settings_content
                            .git
                            .as_ref()?
                            .inline_blame
                            .as_ref()?
                            .enabled
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git
                            .get_or_insert_default()
                            .inline_blame
                            .get_or_insert_default()
                            .enabled = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "延迟",
                description: "显示内联追溯信息之前的延迟。",
                field: Box::new(SettingField {
                    json_path: Some("延迟"),
                    pick: |settings_content| {
                        settings_content
                            .git
                            .as_ref()?
                            .inline_blame
                            .as_ref()?
                            .delay_ms
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git
                            .get_or_insert_default()
                            .inline_blame
                            .get_or_insert_default()
                            .delay_ms = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "内边距",
                description: "源行末尾与内联追溯开头之间的间距(列)。",
                field: Box::new(SettingField {
                    json_path: Some("内边距"),
                    pick: |settings_content| {
                        settings_content
                            .git
                            .as_ref()?
                            .inline_blame
                            .as_ref()?
                            .padding
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git
                            .get_or_insert_default()
                            .inline_blame
                            .get_or_insert_default()
                            .padding = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "最小列",
                description: "显示内联追溯信息的最小列号。",
                field: Box::new(SettingField {
                    json_path: Some("最小列"),
                    pick: |settings_content| {
                        settings_content
                            .git
                            .as_ref()?
                            .inline_blame
                            .as_ref()?
                            .min_column
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git
                            .get_or_insert_default()
                            .inline_blame
                            .get_or_insert_default()
                            .min_column = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示提交摘要",
                description: "将提交摘要作为内联追溯的一部分显示。",
                field: Box::new(SettingField {
                    json_path: Some("显示提交摘要"),
                    pick: |settings_content| {
                        settings_content
                            .git
                            .as_ref()?
                            .inline_blame
                            .as_ref()?
                            .show_commit_summary
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git
                            .get_or_insert_default()
                            .inline_blame
                            .get_or_insert_default()
                            .show_commit_summary = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn git_blame_view_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("Git 追溯视图"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示头像",
                description: "显示提交作者的头像。",
                field: Box::new(SettingField {
                    json_path: Some("显示头像"),
                    pick: |settings_content| {
                        settings_content
                            .git
                            .as_ref()?
                            .blame
                            .as_ref()?
                            .show_avatar
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git
                            .get_or_insert_default()
                            .blame
                            .get_or_insert_default()
                            .show_avatar = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn branch_picker_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("分支选择器"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示作者名称",
                description: "在分支选择器中显示作者名称作为提交信息的一部分。",
                field: Box::new(SettingField {
                    json_path: Some("显示作者名称"),
                    pick: |settings_content| {
                        settings_content
                            .git
                            .as_ref()?
                            .branch_picker
                            .as_ref()?
                            .show_author_name
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git
                            .get_or_insert_default()
                            .branch_picker
                            .get_or_insert_default()
                            .show_author_name = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn git_hunks_section() -> [SettingsPageItem; 4] {
        [
            SettingsPageItem::SectionHeader("Git 代码块"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "代码块样式",
                description: "Git 代码块如何在编辑器中视觉显示。",
                field: Box::new(SettingField {
                    json_path: Some("代码块样式"),
                    pick: |settings_content| settings_content.git.as_ref()?.hunk_style.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.git.get_or_insert_default().hunk_style = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "路径样式",
                description: "在 Git 视图中应优先显示名称还是路径。",
                field: Box::new(SettingField {
                    json_path: Some("路径样式"),
                    pick: |settings_content| settings_content.git.as_ref()?.path_style.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.git.get_or_insert_default().path_style = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示暂存/恢复按钮",
                description: "是否在差异代码块上显示暂存和恢复按钮",
                field: Box::new(SettingField {
                    json_path: Some("git.show_stage_restore_buttons"),
                    pick: |settings_content| {
                        settings_content
                            .git
                            .as_ref()?
                            .show_stage_restore_buttons
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .git
                            .get_or_insert_default()
                            .show_stage_restore_buttons = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    SettingsPage {
        title: "版本控制",
        items: concat_sections![
            git_integration_section(),
            git_gutter_section(),
            inline_git_blame_section(),
            git_blame_view_section(),
            branch_picker_section(),
            git_hunks_section(),
        ],
    }
}

fn collaboration_page() -> SettingsPage {
    fn calls_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("通话"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "加入时静音",
                description: "加入频道或通话时是否静音麦克风。",
                field: Box::new(SettingField {
                    json_path: Some("加入时静音"),
                    pick: |settings_content| settings_content.calls.as_ref()?.mute_on_join.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.calls.get_or_insert_default().mute_on_join = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "加入时共享",
                description: "加入空频道时是否共享当前项目。",
                field: Box::new(SettingField {
                    json_path: Some("加入时共享"),
                    pick: |settings_content| {
                        settings_content.calls.as_ref()?.share_on_join.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.calls.get_or_insert_default().share_on_join = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn audio_settings() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::ActionLink(ActionLink {
                title: "测试音频".into(),
                description: Some("测试您的麦克风和扬声器设置".into()),
                button_text: "测试音频".into(),
                on_click: Arc::new(|_settings_window, window, cx| {
                    open_audio_test_window(window, cx);
                }),
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "输出音频设备",
                description: "选择输出音频设备",
                field: Box::new(SettingField {
                    json_path: Some("audio.experimental.output_audio_device"),
                    pick: |settings_content| {
                        settings_content
                            .audio
                            .as_ref()?
                            .output_audio_device
                            .as_ref()
                            .or(DEFAULT_EMPTY_AUDIO_OUTPUT)
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .audio
                            .get_or_insert_default()
                            .output_audio_device = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "输入音频设备",
                description: "选择输入音频设备",
                field: Box::new(SettingField {
                    json_path: Some("audio.experimental.input_audio_device"),
                    pick: |settings_content| {
                        settings_content
                            .audio
                            .as_ref()?
                            .input_audio_device
                            .as_ref()
                            .or(DEFAULT_EMPTY_AUDIO_INPUT)
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .audio
                            .get_or_insert_default()
                            .input_audio_device = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    SettingsPage {
        title: "协作",
        items: concat_sections![calls_section(), audio_settings()],
    }
}

fn ai_page(cx: &App) -> SettingsPage {
    fn general_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("常规"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "禁用 AI",
                description: "Whether to disable all AI features in VibeDev.",
                field: Box::new(SettingField {
                    json_path: Some("disable_ai"),
                    pick: |settings_content| settings_content.project.disable_ai.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.project.disable_ai = value;
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "对话线程侧边栏位置",
                description: "对话线程侧边栏在窗口中的显示位置",
                field: Box::new(SettingField {
                    json_path: Some("agent.sidebar_side"),
                    pick: |settings_content| settings_content.agent.as_ref()?.sidebar_side.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.agent.get_or_insert_default().sidebar_side = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn agent_configuration_section(_cx: &App) -> Box<[SettingsPageItem]> {
        let mut items = vec![
            SettingsPageItem::SectionHeader("助手配置"),
            SettingsPageItem::SubPageLink(SubPageLink {
                title: "技能".into(),
                r#type: Default::default(),
                json_path: Some("agent.skills"),
                description: Some("查看和管理全局安装或在项目工作树中安装的智能体技能".into()),
                in_json: false,
                files: USER | PROJECT,
                render: render_skills_setup_page,
            }),
            SettingsPageItem::SubPageLink(SubPageLink {
                title: "工具权限".into(),
                r#type: Default::default(),
                json_path: Some("agent.tool_permissions"),
                description: Some("设置正则表达式模式,以便针对特定工具输入自动允许、自动拒绝或始终请求确认。".into()),
                in_json: true,
                files: USER,
                render: render_tool_permissions_setup_page,
            }),
        ];

        items.extend([
            SettingsPageItem::SettingItem(SettingItem {
                title: "单文件审查",
                description: "启用后,助手的编辑也将显示在单文件缓冲区中以供审查。",
                field: Box::new(SettingField {
                    json_path: Some("agent.single_file_review"),
                    pick: |settings_content| {
                        settings_content.agent.as_ref()?.single_file_review.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .single_file_review = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用反馈",
                description: "显示用于对助手编辑进行反馈的投票(点赞/点踩)图标按钮。",
                field: Box::new(SettingField {
                    json_path: Some("agent.enable_feedback"),
                    pick: |settings_content| {
                        settings_content.agent.as_ref()?.enable_feedback.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .enable_feedback = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "助手等待时通知",
                description: "当助手完成响应或需要确认才能运行工具操作时,在哪里显示通知。",
                field: Box::new(SettingField {
                    json_path: Some("agent.notify_when_agent_waiting"),
                    pick: |settings_content| {
                        settings_content
                            .agent
                            .as_ref()?
                            .notify_when_agent_waiting
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .notify_when_agent_waiting = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "助手完成时播放声音",
                description: "当助手完成响应或需要用户输入时何时播放提示音。",
                field: Box::new(SettingField {
                    json_path: Some("agent.play_sound_when_agent_done"),
                    pick: |settings_content| {
                        settings_content
                            .agent
                            .as_ref()?
                            .play_sound_when_agent_done
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .play_sound_when_agent_done = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "展开编辑卡片",
                description: "是否展开助手面板中的编辑卡片,显示差异预览。",
                field: Box::new(SettingField {
                    json_path: Some("agent.expand_edit_card"),
                    pick: |settings_content| {
                        settings_content.agent.as_ref()?.expand_edit_card.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .expand_edit_card = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "展开终端卡片",
                description: "是否展开助手面板中的终端卡片,显示完整的命令输出。",
                field: Box::new(SettingField {
                    json_path: Some("agent.expand_terminal_card"),
                    pick: |settings_content| {
                        settings_content
                            .agent
                            .as_ref()?
                            .expand_terminal_card
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .expand_terminal_card = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "思考过程显示",
                description: "默认情况下如何显示思考块。'Auto' 在流式输出时完全展开, 完成后自动折叠。'Preview' 在流式输出时自动展开但有高度限制。'Always Expanded' 显示完整内容。'Always Collapsed' 保持折叠状态。",
                field: Box::new(SettingField {
                    json_path: Some("agent.thinking_display"),
                    pick: |settings_content| {
                        settings_content
                            .agent
                            .as_ref()?
                            .thinking_display
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .thinking_display = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "终端停止时取消生成",
                description: "点击正在运行的终端工具上的停止按钮是否也应取消助手的生成。请注意,这仅适用于停止按钮,不适用于终端内的 ctrl+c。",
                field: Box::new(SettingField {
                    json_path: Some("agent.cancel_generation_on_terminal_stop"),
                    pick: |settings_content| {
                        settings_content
                            .agent
                            .as_ref()?
                            .cancel_generation_on_terminal_stop
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .cancel_generation_on_terminal_stop = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用修饰键发送",
                description: "是否始终使用 cmd-enter(或在 Linux 或 Windows 上使用 ctrl-enter)发送消息。",
                field: Box::new(SettingField {
                    json_path: Some("agent.use_modifier_to_send"),
                    pick: |settings_content| {
                        settings_content
                            .agent
                            .as_ref()?
                            .use_modifier_to_send
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .use_modifier_to_send = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "消息编辑器最小行数",
                description: "助手消息编辑器中显示的最小行数。",
                field: Box::new(SettingField {
                    json_path: Some("agent.message_editor_min_lines"),
                    pick: |settings_content| {
                        settings_content
                            .agent
                            .as_ref()?
                            .message_editor_min_lines
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .message_editor_min_lines = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示轮次统计",
                description: "是否显示轮次统计信息,例如生成期间的经过时间和最终轮次持续时间。",
                field: Box::new(SettingField {
                    json_path: Some("agent.show_turn_stats"),
                    pick: |settings_content| {
                        settings_content.agent.as_ref()?.show_turn_stats.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .show_turn_stats = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示合并冲突指示器",
                description: "是否在状态栏显示合并冲突指示器, 以提供使用助手解决冲突的选项。",
                field: Box::new(SettingField {
                    json_path: Some("agent.show_merge_conflict_indicator"),
                    pick: |settings_content| {
                        settings_content.agent.as_ref()?.show_merge_conflict_indicator.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .agent
                            .get_or_insert_default()
                            .show_merge_conflict_indicator = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]);

        items.into_boxed_slice()
    }

    fn context_servers_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("上下文服务器"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "上下文服务器超时",
                description: "上下文服务器工具调用的默认超时时间(秒)。可以在 context_servers 配置中为每个服务器覆盖。",
                field: Box::new(SettingField {
                    json_path: Some("context_server_timeout"),
                    pick: |settings_content| {
                        settings_content.project.context_server_timeout.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.project.context_server_timeout = value;
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn edit_prediction_display_sub_section() -> [SettingsPageItem; 1] {
        [SettingsPageItem::SettingItem(SettingItem {
            title: "显示模式",
            description: "何时在缓冲区中显示编辑预测预览。eager 模式内联显示它们,而 subtle 模式仅在按住修饰键时显示它们。",
            field: Box::new(SettingField {
                json_path: Some("edit_prediction.display_mode"),
                pick: |settings_content| {
                    settings_content
                        .project
                        .all_languages
                        .edit_predictions
                        .as_ref()?
                        .mode
                        .as_ref()
                },
                write: |settings_content, value, _| {
                    settings_content
                        .project
                        .all_languages
                        .edit_predictions
                        .get_or_insert_default()
                        .mode = value;
                },
            }),
            metadata: None,
            files: USER,
        })]
    }

    SettingsPage {
        title: "AI",
        items: concat_sections![
            general_section(),
            agent_configuration_section(cx),
            context_servers_section(),
            edit_prediction_language_settings_section(),
            edit_prediction_display_sub_section()
        ],
    }
}

fn network_page() -> SettingsPage {
    fn network_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SectionHeader("网络"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "代理",
                description: "用于网络请求的代理。",
                field: Box::new(SettingField {
                    json_path: Some("proxy"),
                    pick: |settings_content| settings_content.proxy.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.proxy = value;
                    },
                }),
                metadata: Some(Box::new(SettingsFieldMetadata {
                    placeholder: Some("socks5h://localhost:10808"),
                    ..Default::default()
                })),
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "服务器 URL",
                description: "The URL of the VibeDev server to connect to.",
                field: Box::new(SettingField {
                    json_path: Some("server_url"),
                    pick: |settings_content| settings_content.server_url.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.server_url = value;
                    },
                }),
                metadata: Some(Box::new(SettingsFieldMetadata {
                    placeholder: Some("https://zed.dev"),
                    ..Default::default()
                })),
                files: USER,
            }),
        ]
    }

    SettingsPage {
        title: "网络",
        items: concat_sections![network_section()],
    }
}

fn language_settings_field<T>(
    settings_content: &SettingsContent,
    get_language_setting_field: fn(&LanguageSettingsContent) -> Option<&T>,
) -> Option<&T> {
    let all_languages = &settings_content.project.all_languages;

    active_language()
        .and_then(|current_language_name| {
            all_languages
                .languages
                .0
                .get(current_language_name.as_ref())
        })
        .and_then(get_language_setting_field)
        .or_else(|| get_language_setting_field(&all_languages.defaults))
}

fn language_settings_field_mut<T>(
    settings_content: &mut SettingsContent,
    value: Option<T>,
    write: fn(&mut LanguageSettingsContent, Option<T>),
) {
    let all_languages = &mut settings_content.project.all_languages;
    let language_content = if let Some(current_language) = active_language() {
        all_languages
            .languages
            .0
            .entry(current_language.to_string())
            .or_default()
    } else {
        &mut all_languages.defaults
    };
    write(language_content, value);
}

fn language_settings_data() -> Box<[SettingsPageItem]> {
    fn indentation_section() -> [SettingsPageItem; 5] {
        [
            SettingsPageItem::SectionHeader("缩进"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "制表符宽度",
                description: "一个制表符应占用的列数。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).tab_size"), // TODO(cameron): not JQ syntax because not URL-safe
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.tab_size.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.tab_size = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "硬制表符",
                description: "是否使用制表符缩进行,而不是使用多个空格。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).hard_tabs"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.hard_tabs.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.hard_tabs = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动缩进",
                description: "控制输入时的自动缩进行为。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).auto_indent"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.auto_indent.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.auto_indent = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "粘贴时自动缩进",
                description: "是否应根据上下文调整粘贴内容的缩进。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).auto_indent_on_paste"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.auto_indent_on_paste.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.auto_indent_on_paste = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn wrapping_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("换行"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "软换行",
                description: "如何对长文本行进行软换行。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).soft_wrap"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.soft_wrap.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.soft_wrap = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示自动换行辅助线",
                description: "在编辑器中显示换行参考线。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).show_wrap_guides"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.show_wrap_guides.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.show_wrap_guides = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "首选行长",
                description: "软换行的列位置,针对启用软换行的缓冲区。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).preferred_line_length"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.preferred_line_length.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.preferred_line_length = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动换行辅助线",
                description: "在编辑器中显示换行参考线的字符计数。",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).wrap_guides"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.wrap_guides.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.wrap_guides = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "允许重新换行",
                description: "Controls where the `editor::rewrap` action is allowed for this language.",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).allow_rewrap"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.allow_rewrap.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.allow_rewrap = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn indent_guides_section() -> [SettingsPageItem; 6] {
        [
            SettingsPageItem::SectionHeader("缩进参考线"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用",
                description: "在编辑器中显示缩进参考线。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).indent_guides.enabled"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language
                                .indent_guides
                                .as_ref()
                                .and_then(|indent_guides| indent_guides.enabled.as_ref())
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.indent_guides.get_or_insert_default().enabled = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "线宽",
                description: "缩进参考线的宽度(像素),介于 1 到 10 之间。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).indent_guides.line_width"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language
                                .indent_guides
                                .as_ref()
                                .and_then(|indent_guides| indent_guides.line_width.as_ref())
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.indent_guides.get_or_insert_default().line_width = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "活动线宽",
                description: "活动缩进参考线的宽度(像素),介于 1 到 10 之间。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).indent_guides.active_line_width"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language
                                .indent_guides
                                .as_ref()
                                .and_then(|indent_guides| indent_guides.active_line_width.as_ref())
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language
                                .indent_guides
                                .get_or_insert_default()
                                .active_line_width = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "着色",
                description: "确定缩进参考线的着色方式。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).indent_guides.coloring"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language
                                .indent_guides
                                .as_ref()
                                .and_then(|indent_guides| indent_guides.coloring.as_ref())
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.indent_guides.get_or_insert_default().coloring = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "背景着色",
                description: "确定缩进参考线背景的着色方式。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).indent_guides.background_coloring"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.indent_guides.as_ref().and_then(|indent_guides| {
                                indent_guides.background_coloring.as_ref()
                            })
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language
                                .indent_guides
                                .get_or_insert_default()
                                .background_coloring = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn formatting_section() -> [SettingsPageItem; 8] {
        [
            SettingsPageItem::SectionHeader("格式化"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "保存时格式化",
                description: "是否在保存前执行缓冲区格式化。",
                field: Box::new(
                    // TODO(settings_ui): this setting should just be a bool
                    SettingField {
                        json_path: Some("languages.$(language).format_on_save"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.format_on_save.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.format_on_save = value;
                                },
                            )
                        },
                    },
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "保存时移除尾随空格",
                description: "是否在保存缓冲区之前移除各行中任何尾随空格。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).remove_trailing_whitespace_on_save"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.remove_trailing_whitespace_on_save.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.remove_trailing_whitespace_on_save = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "保存时确保末尾换行",
                description: "是否在保存缓冲区时确保其末尾有一个换行符。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).ensure_final_newline_on_save"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.ensure_final_newline_on_save.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.ensure_final_newline_on_save = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "行尾符",
                description: "新文件以及格式化和保存操作时如何处理行尾符。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).line_ending"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.line_ending.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.line_ending = value;
                        })
                    },
                }),
                metadata: Some(Box::new(SettingsFieldMetadata {
                    should_do_titlecase: Some(false),
                    ..Default::default()
                })),
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "格式化工具",
                description: "如何执行缓冲区格式化。",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).formatter"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.formatter.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.formatter = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用输入时格式化",
                description: "是否在每次输入由 LSP 服务器能力定义的“触发”符号后,使用额外的 LSP 查询来格式化(并修正)代码",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).use_on_type_format"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.use_on_type_format.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.use_on_type_format = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "格式化时的代码操作",
                description: "格式化时要运行的其他代码操作。",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).code_actions_on_format"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.code_actions_on_format.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.code_actions_on_format = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn autoclose_section() -> [SettingsPageItem; 5] {
        [
            SettingsPageItem::SectionHeader("自动闭合"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用自动闭合",
                description: "Whether to automatically type closing characters for you. For example, when you type '(', VibeDev will automatically add a closing ')' at the correct position.",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).use_autoclose"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.use_autoclose.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.use_autoclose = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "使用自动包围",
                description: "Whether to automatically surround text with characters for you. For example, when you select text and type '(', VibeDev will automatically surround text with ().",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).use_auto_surround"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.use_auto_surround.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.use_auto_surround = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "始终将括号视为自动闭合",
                description: "控制是否始终跳过并自动删除闭合字符,无论它们是如何插入的。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).always_treat_brackets_as_autoclosed"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.always_treat_brackets_as_autoclosed.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.always_treat_brackets_as_autoclosed = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "JSX 标签自动闭合",
                description: "是否自动闭合 JSX 标签。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).jsx_tag_auto_close"),
                    // TODO(settings_ui): this setting should just be a bool
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.jsx_tag_auto_close.as_ref()?.enabled.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.jsx_tag_auto_close.get_or_insert_default().enabled = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn whitespace_section() -> [SettingsPageItem; 4] {
        [
            SettingsPageItem::SectionHeader("空白字符"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示空白字符",
                description: "是否在编辑器中显示制表符和空格。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).show_whitespaces"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.show_whitespaces.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.show_whitespaces = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "空格指示符",
                description: "当启用 show_whitespaces 时用于渲染空格字符的可见字符(默认值:“•”)",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).whitespace_map.space"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.whitespace_map.as_ref()?.space.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.whitespace_map.get_or_insert_default().space = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "制表符指示符",
                description: "当启用 show_whitespaces 时用于渲染制表符字符的可见字符(默认值:“→”)",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).whitespace_map.tab"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.whitespace_map.as_ref()?.tab.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.whitespace_map.get_or_insert_default().tab = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn completions_section() -> [SettingsPageItem; 8] {
        [
            SettingsPageItem::SectionHeader("Completions"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "输入时显示补全",
                description: "是否在编辑器中输入时自动弹出补全菜单,而无需显式请求。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).show_completions_on_input"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.show_completions_on_input.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.show_completions_on_input = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示补全文档",
                description: "是否在补全菜单中显示内联和侧边文档。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).show_completion_documentation"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.show_completion_documentation.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.show_completion_documentation = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "单词",
                description: "控制如何补全单词。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).completions.words"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.completions.as_ref()?.words.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.completions.get_or_insert_default().words = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "单词最小长度",
                description: "补全查询中必须包含多少个字符才能自动显示基于单词的补全。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).completions.words_min_length"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.completions.as_ref()?.words_min_length.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language
                                .completions
                                .get_or_insert_default()
                                .words_min_length = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "补全菜单滚动条",
                description: "何时在补全菜单中显示滚动条。",
                field: Box::new(SettingField {
                    json_path: Some("editor.completion_menu_scrollbar"),
                    pick: |settings_content| {
                        settings_content.editor.completion_menu_scrollbar.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.completion_menu_scrollbar = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "补全详情对齐方式",
                description: "是否将代码补全上下文菜单中的详情文本左对齐或右对齐。",
                field: Box::new(SettingField {
                    json_path: Some("editor.completion_detail_alignment"),
                    pick: |settings_content| {
                        settings_content.editor.completion_detail_alignment.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.completion_detail_alignment = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "补全菜单项类型",
                description: "如何显示补全菜单中每个条目的 LSP 项目类型(函数、方法、变量等)",
                field: Box::new(SettingField {
                    json_path: Some("editor.completion_menu_item_kind"),
                    pick: |settings_content| {
                        settings_content.editor.completion_menu_item_kind.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.completion_menu_item_kind = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    fn inlay_hints_section() -> [SettingsPageItem; 10] {
        [
            SettingsPageItem::SectionHeader("内联提示"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用",
                description: "全局开关,用于开启或关闭提示。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).inlay_hints.enabled"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.inlay_hints.as_ref()?.enabled.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.inlay_hints.get_or_insert_default().enabled = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示值提示",
                description: "全局开关,用于在调试时开启或关闭内联值。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).inlay_hints.show_value_hints"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.inlay_hints.as_ref()?.show_value_hints.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language
                                .inlay_hints
                                .get_or_insert_default()
                                .show_value_hints = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示类型提示",
                description: "是否应显示类型提示。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).inlay_hints.show_type_hints"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.inlay_hints.as_ref()?.show_type_hints.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.inlay_hints.get_or_insert_default().show_type_hints = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示参数提示",
                description: "是否应显示参数提示。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).inlay_hints.show_parameter_hints"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.inlay_hints.as_ref()?.show_parameter_hints.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language
                                .inlay_hints
                                .get_or_insert_default()
                                .show_parameter_hints = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示其他提示",
                description: "是否应显示其他提示。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).inlay_hints.show_other_hints"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.inlay_hints.as_ref()?.show_other_hints.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language
                                .inlay_hints
                                .get_or_insert_default()
                                .show_other_hints = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "显示背景",
                description: "为内联提示显示背景。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).inlay_hints.show_background"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.inlay_hints.as_ref()?.show_background.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.inlay_hints.get_or_insert_default().show_background = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "编辑防抖毫秒数",
                description: "是否在缓冲区编辑后对内联提示更新进行防抖(设置为 0 以禁用防抖)。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).inlay_hints.edit_debounce_ms"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.inlay_hints.as_ref()?.edit_debounce_ms.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language
                                .inlay_hints
                                .get_or_insert_default()
                                .edit_debounce_ms = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "滚动防抖毫秒数",
                description: "是否在缓冲区滚动后对内联提示更新进行防抖(设置为 0 以禁用防抖)。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).inlay_hints.scroll_debounce_ms"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.inlay_hints.as_ref()?.scroll_debounce_ms.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language
                                .inlay_hints
                                .get_or_insert_default()
                                .scroll_debounce_ms = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "按下修饰键时切换",
                description: "当用户按下指定的修饰键时切换内联提示(隐藏或显示)。",
                field: Box::new(
                    SettingField {
                        json_path: Some(
                            "languages.$(language).inlay_hints.toggle_on_modifiers_press",
                        ),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language
                                    .inlay_hints
                                    .as_ref()?
                                    .toggle_on_modifiers_press
                                    .as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language
                                        .inlay_hints
                                        .get_or_insert_default()
                                        .toggle_on_modifiers_press = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn tasks_section() -> [SettingsPageItem; 4] {
        [
            SettingsPageItem::SectionHeader("任务"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用",
                description: "是否已启用此语言的任务。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).tasks.enabled"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.tasks.as_ref()?.enabled.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.tasks.get_or_insert_default().enabled = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "变量",
                description: "为特定语言设置的其他任务变量。",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).tasks.variables"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.tasks.as_ref()?.variables.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.tasks.get_or_insert_default().variables = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "优先使用 LSP",
                description: "Use LSP tasks over VibeDev language extension tasks.",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).tasks.prefer_lsp"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.tasks.as_ref()?.prefer_lsp.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.tasks.get_or_insert_default().prefer_lsp = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn miscellaneous_section() -> [SettingsPageItem; 7] {
        [
            SettingsPageItem::SectionHeader("杂项"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用单词差异",
                description: "是否在编辑器中启用单词差异高亮。启用后,修改行内更改的单词将被高亮显示,以准确显示更改内容。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).word_diff_enabled"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.word_diff_enabled.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.word_diff_enabled = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "调试器",
                description: "此语言的首选调试器。",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).debuggers"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.debuggers.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.debuggers = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "中键粘贴",
                description: "在 Linux 上启用中键粘贴。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).editor.middle_click_paste"),
                    pick: |settings_content| settings_content.editor.middle_click_paste.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.editor.middle_click_paste = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "换行时延续注释",
                description: "当上一行也是注释时,是否在新行开始处添加注释。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).extend_comment_on_newline"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.extend_comment_on_newline.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.extend_comment_on_newline = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "括号着色",
                description: "是否在编辑器中为括号着色。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).colorize_brackets"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.colorize_brackets.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.colorize_brackets = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "Vim/Emacs 模式行支持",
                description: "搜索模式行的行数(设为 0 以禁用)。",
                field: Box::new(SettingField {
                    json_path: Some("modeline_lines"),
                    pick: |settings_content| settings_content.modeline_lines.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.modeline_lines = value;
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn global_only_miscellaneous_sub_section() -> [SettingsPageItem; 3] {
        [
            SettingsPageItem::SettingItem(SettingItem {
                title: "图片查看器",
                description: "图片文件大小的单位。",
                field: Box::new(SettingField {
                    json_path: Some("image_viewer.unit"),
                    pick: |settings_content| {
                        settings_content
                            .image_viewer
                            .as_ref()
                            .and_then(|image_viewer| image_viewer.unit.as_ref())
                    },
                    write: |settings_content, value, _| {
                        settings_content.image_viewer.get_or_insert_default().unit = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "自动替换 Emoji 短代码",
                description: "是否自动将 emoji 短代码替换为 emoji 字符。",
                field: Box::new(SettingField {
                    json_path: Some("message_editor.auto_replace_emoji_shortcode"),
                    pick: |settings_content| {
                        settings_content
                            .message_editor
                            .as_ref()
                            .and_then(|message_editor| {
                                message_editor.auto_replace_emoji_shortcode.as_ref()
                            })
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .message_editor
                            .get_or_insert_default()
                            .auto_replace_emoji_shortcode = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "放置目标大小",
                description: "编辑器中放置目标的相对大小,将作为拆分窗格打开拖放的文件。",
                field: Box::new(SettingField {
                    json_path: Some("drop_target_size"),
                    pick: |settings_content| settings_content.workspace.drop_target_size.as_ref(),
                    write: |settings_content, value, _| {
                        settings_content.workspace.drop_target_size = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
        ]
    }

    let is_global = active_language().is_none();

    let code_lens_item = [SettingsPageItem::SettingItem(SettingItem {
        title: "代码透镜",
        description: "是否以及如何显示语言服务器的代码透镜。",
        field: Box::new(SettingField {
            json_path: Some("code_lens"),
            pick: |settings_content| settings_content.editor.code_lens.as_ref(),
            write: |settings_content, value, _| {
                settings_content.editor.code_lens = value;
            },
        }),
        metadata: None,
        files: USER,
    })];

    let lsp_document_colors_item = [SettingsPageItem::SettingItem(SettingItem {
        title: "LSP 文档颜色",
        description: "如何在编辑器中渲染 LSP 颜色预览。",
        field: Box::new(SettingField {
            json_path: Some("lsp_document_colors"),
            pick: |settings_content| settings_content.editor.lsp_document_colors.as_ref(),
            write: |settings_content, value, _| {
                settings_content.editor.lsp_document_colors = value;
            },
        }),
        metadata: None,
        files: USER,
    })];

    if is_global {
        concat_sections!(
            indentation_section(),
            wrapping_section(),
            indent_guides_section(),
            formatting_section(),
            autoclose_section(),
            whitespace_section(),
            completions_section(),
            inlay_hints_section(),
            code_lens_item,
            lsp_document_colors_item,
            tasks_section(),
            miscellaneous_section(),
            global_only_miscellaneous_sub_section(),
        )
    } else {
        concat_sections!(
            indentation_section(),
            wrapping_section(),
            indent_guides_section(),
            formatting_section(),
            autoclose_section(),
            whitespace_section(),
            completions_section(),
            inlay_hints_section(),
            code_lens_item,
            tasks_section(),
            miscellaneous_section(),
        )
    }
}

/// LanguageSettings items that should be included in the "语言和工具" page
/// not the "编辑器" page
fn non_editor_language_settings_data() -> Box<[SettingsPageItem]> {
    fn lsp_section() -> [SettingsPageItem; 9] {
        [
            SettingsPageItem::SectionHeader("LSP"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用语言服务器",
                description: "是否使用语言服务器来提供代码智能。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).enable_language_server"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.enable_language_server.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.enable_language_server = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "语言服务器",
                description: "要为此语言使用(或禁用)的语言服务器列表。",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).language_servers"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.language_servers.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.language_servers = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "关联编辑",
                description: "如果语言服务器支持,是否对关联范围执行关联编辑。例如,当编辑开始标签 <html> 时,结束标签 </html> 的内容也会被编辑。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).linked_edits"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.linked_edits.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.linked_edits = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "转到定义回退",
                description: "是否在语言服务器返回空的“转到定义”响应时进行后续处理。",
                field: Box::new(SettingField {
                    json_path: Some("go_to_definition_fallback"),
                    pick: |settings_content| {
                        settings_content.editor.go_to_definition_fallback.as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.go_to_definition_fallback = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "转到定义滚动策略",
                description: "导航到定义或引用时如何滚动目标到视图中。",
                field: Box::new(SettingField {
                    json_path: Some("go_to_definition_scroll_strategy"),
                    pick: |settings_content| {
                        settings_content
                            .editor
                            .go_to_definition_scroll_strategy
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content.editor.go_to_definition_scroll_strategy = value;
                    },
                }),
                metadata: None,
                files: USER,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "语义标记",
                description: {
                    static DESCRIPTION: OnceLock<&'static str> = OnceLock::new();
                    DESCRIPTION.get_or_init(|| {
                        SemanticTokens::VARIANTS
                            .iter()
                            .filter_map(|v| {
                                v.get_documentation().map(|doc| format!("{v:?}: {doc}"))
                            })
                            .join("\n")
                            .leak()
                    })
                },
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).semantic_tokens"),
                    pick: |settings_content| {
                        settings_content
                            .project
                            .all_languages
                            .defaults
                            .semantic_tokens
                            .as_ref()
                    },
                    write: |settings_content, value, _| {
                        settings_content
                            .project
                            .all_languages
                            .defaults
                            .semantic_tokens = value;
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "LSP 折叠范围",
                description: "启用后,使用来自语言服务器的折叠范围,而不是基于缩进的折叠",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).document_folding_ranges"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.document_folding_ranges.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.document_folding_ranges = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "LSP 文档符号",
                description: "启用时,使用语言服务器的文档符号来显示大纲和面包屑导航,而非 tree-sitter。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).document_symbols"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.document_symbols.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.document_symbols = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn lsp_completions_section() -> [SettingsPageItem; 4] {
        [
            SettingsPageItem::SectionHeader("LSP 补全"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "启用",
                description: "是否获取 LSP 补全。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).completions.lsp"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.completions.as_ref()?.lsp.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.completions.get_or_insert_default().lsp = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "获取超时(毫秒)",
                description: "获取 LSP 补全时,确定等待特定服务器响应的时间(设置为 0 表示无限期等待)。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).completions.lsp_fetch_timeout_ms"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.completions.as_ref()?.lsp_fetch_timeout_ms.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language
                                .completions
                                .get_or_insert_default()
                                .lsp_fetch_timeout_ms = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "插入模式",
                description: "控制如何插入 LSP 补全。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).completions.lsp_insert_mode"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.completions.as_ref()?.lsp_insert_mode.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.completions.get_or_insert_default().lsp_insert_mode = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn debugger_section() -> [SettingsPageItem; 2] {
        [
            SettingsPageItem::SectionHeader("调试器"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "调试器",
                description: "此语言的首选调试器。",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).debuggers"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.debuggers.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.debuggers = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    fn prettier_section() -> [SettingsPageItem; 5] {
        [
            SettingsPageItem::SectionHeader("Prettier"),
            SettingsPageItem::SettingItem(SettingItem {
                title: "允许",
                description: "启用或禁用给定语言的 Prettier 格式化。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).prettier.allowed"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.prettier.as_ref()?.allowed.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.prettier.get_or_insert_default().allowed = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "解析器",
                description: "强制 Prettier 集成在使用该语言格式化文件时使用特定的解析器名称。",
                field: Box::new(SettingField {
                    json_path: Some("languages.$(language).prettier.parser"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.prettier.as_ref()?.parser.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.prettier.get_or_insert_default().parser = value;
                        })
                    },
                }),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "插件",
                description: "强制 Prettier 集成在使用该语言格式化文件时使用特定的插件。",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).prettier.plugins"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.prettier.as_ref()?.plugins.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.prettier.get_or_insert_default().plugins = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
            SettingsPageItem::SettingItem(SettingItem {
                title: "选项",
                description: "默认的 Prettier 选项,格式与 package.json 中的 Prettier 部分相同。",
                field: Box::new(
                    SettingField {
                        json_path: Some("languages.$(language).prettier.options"),
                        pick: |settings_content| {
                            language_settings_field(settings_content, |language| {
                                language.prettier.as_ref()?.options.as_ref()
                            })
                        },
                        write: |settings_content, value, _| {
                            language_settings_field_mut(
                                settings_content,
                                value,
                                |language, value| {
                                    language.prettier.get_or_insert_default().options = value;
                                },
                            )
                        },
                    }
                    .unimplemented(),
                ),
                metadata: None,
                files: USER | PROJECT,
            }),
        ]
    }

    concat_sections!(
        lsp_section(),
        lsp_completions_section(),
        debugger_section(),
        prettier_section(),
    )
}

fn edit_prediction_language_settings_section() -> [SettingsPageItem; 5] {
    [
        SettingsPageItem::SectionHeader("编辑预测"),
        SettingsPageItem::SubPageLink(SubPageLink {
            title: "配置提供商".into(),
            r#type: Default::default(),
            json_path: Some("edit_predictions.providers"),
            description: Some("Set up different edit prediction providers in complement to VibeDev's built-in Zeta model.".into()),
            in_json: false,
            files: USER,
            render: render_edit_prediction_setup_page
        }),
        SettingsPageItem::SettingItem(SettingItem {
            title: "数据收集",
            description: "Controls whether VibeDev may collect training data when using VibeDev's Edit Predictions. Data is only collected for files in projects detected as open source. The default value uses the preference previously set via the status-bar toggle, or false if no preference has been stored.",
            field: Box::new(SettingField {
                json_path: Some("edit_predictions.allow_data_collection"),
                pick: |settings_content| {
                    settings_content
                        .project
                        .all_languages
                        .edit_predictions
                        .as_ref()?
                        .allow_data_collection
                        .as_ref()
                },
                write: |settings_content, value, _app| {
                    settings_content
                        .project
                        .all_languages
                        .edit_predictions
                        .get_or_insert_default()
                        .allow_data_collection = value;
                },
            }),
            metadata: None,
            files: USER,
        }),
        SettingsPageItem::SettingItem(SettingItem {
            title: "显示编辑预测",
            description: "控制是立即还是手动显示编辑预测。",
            field: Box::new(SettingField {
                json_path: Some("languages.$(language).show_edit_predictions"),
                pick: |settings_content| {
                    language_settings_field(settings_content, |language| {
                        language.show_edit_predictions.as_ref()
                    })
                },
                write: |settings_content, value, _| {
                    language_settings_field_mut(settings_content, value, |language, value| {
                        language.show_edit_predictions = value;
                    })
                },
            }),
            metadata: None,
            files: USER | PROJECT,
        }),
        SettingsPageItem::SettingItem(SettingItem {
            title: "在语言作用域中禁用",
            description: "控制是否在给定的语言作用域中显示编辑预测。",
            field: Box::new(
                SettingField {
                    json_path: Some("languages.$(language).edit_predictions_disabled_in"),
                    pick: |settings_content| {
                        language_settings_field(settings_content, |language| {
                            language.edit_predictions_disabled_in.as_ref()
                        })
                    },
                    write: |settings_content, value, _| {
                        language_settings_field_mut(settings_content, value, |language, value| {
                            language.edit_predictions_disabled_in = value;
                        })
                    },
                }
                .unimplemented(),
            ),
            metadata: None,
            files: USER | PROJECT,
        }),
    ]
}

fn show_scrollbar_or_editor(
    settings_content: &SettingsContent,
    show: fn(&SettingsContent) -> Option<&settings::ShowScrollbar>,
) -> Option<&settings::ShowScrollbar> {
    show(settings_content).or(settings_content
        .editor
        .scrollbar
        .as_ref()
        .and_then(|scrollbar| scrollbar.show.as_ref()))
}

fn dynamic_variants<T>() -> &'static [T::Discriminant]
where
    T: strum::IntoDiscriminant,
    T::Discriminant: strum::VariantArray,
{
    <<T as strum::IntoDiscriminant>::Discriminant as strum::VariantArray>::VARIANTS
}

/// Updates the `vim_mode` setting, disabling `helix_mode` if present and
/// `vim_mode` is being enabled.
fn write_vim_mode(settings: &mut SettingsContent, value: Option<bool>, _: &App) {
    write_vim_mode_inner(settings, value);
}

fn write_vim_mode_inner(settings: &mut SettingsContent, value: Option<bool>) {
    if value == Some(true) && settings.helix_mode == Some(true) {
        settings.helix_mode = Some(false);
    }
    settings.vim_mode = value;
}

/// Updates the `helix_mode` setting, disabling `vim_mode` if present and
/// `helix_mode` is being enabled.
fn write_helix_mode(settings: &mut SettingsContent, value: Option<bool>, _: &App) {
    write_helix_mode_inner(settings, value);
}

fn write_helix_mode_inner(settings: &mut SettingsContent, value: Option<bool>) {
    if value == Some(true) && settings.vim_mode == Some(true) {
        settings.vim_mode = Some(false);
    }
    settings.helix_mode = value;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_vim_helix_mode() {
        // Enabling vim mode while `vim_mode` and `helix_mode` are not yet set
        // should only update the `vim_mode` setting.
        let mut settings = SettingsContent::default();
        write_vim_mode_inner(&mut settings, Some(true));
        assert_eq!(settings.vim_mode, Some(true));
        assert_eq!(settings.helix_mode, None);

        // Enabling helix mode while `vim_mode` and `helix_mode` are not yet set
        // should only update the `helix_mode` setting.
        let mut settings = SettingsContent::default();
        write_helix_mode_inner(&mut settings, Some(true));
        assert_eq!(settings.helix_mode, Some(true));
        assert_eq!(settings.vim_mode, None);

        // Disabling helix mode should only touch `helix_mode` setting when
        // `vim_mode` is not set.
        write_helix_mode_inner(&mut settings, Some(false));
        assert_eq!(settings.helix_mode, Some(false));
        assert_eq!(settings.vim_mode, None);

        // Enabling vim mode should update `vim_mode` but leave `helix_mode`
        // untouched.
        write_vim_mode_inner(&mut settings, Some(true));
        assert_eq!(settings.vim_mode, Some(true));
        assert_eq!(settings.helix_mode, Some(false));

        // Enabling helix mode should update `helix_mode` and disable
        // `vim_mode`.
        write_helix_mode_inner(&mut settings, Some(true));
        assert_eq!(settings.helix_mode, Some(true));
        assert_eq!(settings.vim_mode, Some(false));

        // Enabling vim mode should update `vim_mode` and disable
        // `helix_mode`.
        write_vim_mode_inner(&mut settings, Some(true));
        assert_eq!(settings.vim_mode, Some(true));
        assert_eq!(settings.helix_mode, Some(false));
    }
}
