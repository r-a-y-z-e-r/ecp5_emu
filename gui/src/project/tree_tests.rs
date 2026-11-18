use super::*;

#[test]
fn examples_filter_with_the_catalogue_and_tag_dev_entries() {
    let root = crate::test_temp::demo_fixture();
    let mut catalogue = ecp5_emu::flow::demo_copy::catalogue_in(&root.join("examples")).unwrap();
    let examples: Vec<_> = catalogue
        .demos
        .iter()
        .map(|d| (d.name.clone(), root.join("examples").join(&d.name)))
        .collect();
    let ctx = egui::Context::default();
    for dev in [false, true, false] {
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            example_choices(
                ui,
                &examples,
                &catalogue,
                dev,
                &super::super::DisplayPaths::new(Some(&root)),
            );
        });
        let text = crate::test_temp::painted_text(&output);
        assert!(text.iter().any(|(s, _)| s == "ulx3s/hero"));
        assert_eq!(
            text.iter().any(|(s, _)| s == "ulx3s/hardware"),
            dev,
            "{text:?}"
        );
        assert_eq!(text.iter().any(|(s, _)| s == "dev"), dev, "{text:?}");
    }
    for demo in &mut catalogue.demos {
        demo.showcase = false;
    }
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        example_choices(
            ui,
            &examples,
            &catalogue,
            false,
            &super::super::DisplayPaths::new(Some(&root)),
        );
    });
    assert!(crate::test_temp::painted_text(&output)
        .iter()
        .any(|(s, _)| s == crate::start::NO_SHOWCASE));
    fs::remove_dir_all(root).unwrap();
}

struct TreeUi {
    ctx: egui::Context,
    root: PathBuf,
    project: Result<Project, String>,
    editor: Editor,
    path_input: String,
    time: f64,
    busy: bool,
}

impl TreeUi {
    fn new(theme: egui::ThemePreference) -> Self {
        let root = crate::test_temp::unique_dir("ecp5-tree-rows-")
            .canonicalize()
            .unwrap();
        fs::create_dir(root.join("rtl")).unwrap();
        fs::create_dir(root.join("build")).unwrap();
        fs::write(
            root.join("ecp5.toml"),
            "[sources]\nverilog = ['rtl/top.v']\n",
        )
        .unwrap();
        fs::write(root.join("rtl/top.v"), "module top; endmodule").unwrap();
        fs::write(root.join("top.lpf"), "").unwrap();
        fs::write(root.join("build/yosys.log"), "tool output\n").unwrap();
        fs::write(root.join("build/top.config"), "config").unwrap();
        let project = Ok(Project::open(&root).unwrap());
        let ctx = egui::Context::default();
        crate::theme::install(&ctx, theme);
        ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
        Self {
            ctx,
            path_input: root.display().to_string(),
            root,
            project,
            editor: Editor::default(),
            time: 0.0,
            busy: false,
        }
    }

    fn frame(
        &mut self,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<TreeAction>, egui::Rect) {
        // Keep press/release pairs shorter than egui's maximum click duration.
        self.time += 0.1;
        let steps = self
            .project
            .as_ref()
            .map_or_else(|_| Vec::new(), default_steps);
        let mut action = None;
        let mut panel = egui::Rect::NOTHING;
        let output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ui| {
                panel = egui::Panel::left("project_tree")
                    .default_size(260.0)
                    .show(ui, |ui| {
                        action = show(
                            ui,
                            &mut self.path_input,
                            &self.project,
                            &steps,
                            &[],
                            false,
                            &self.editor,
                            &[],
                            self.busy,
                            &super::super::DisplayPaths::new(Some(&self.root)),
                        );
                    })
                    .response
                    .rect;
            },
        );
        (output, action, panel)
    }

    fn click(&mut self, pos: egui::Pos2) -> Option<TreeAction> {
        let mut action = None;
        for pressed in [true, false] {
            let (_, clicked, _) = self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            action = action.or(clicked);
        }
        action
    }
}

impl Drop for TreeUi {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn text_rect(output: &egui::FullOutput, label: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing text: {label}"))
}

#[test]
fn file_rows_open_the_same_file_from_text_indent_empty_space_and_size() {
    for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
        for (label, relative, read_only) in [
            ("rtl/top.v", "rtl/top.v", false),
            ("yosys.log", "build/yosys.log", true),
        ] {
            for target in ["text", "indent", "empty right", "right edge", "size"] {
                if target == "size" && !read_only {
                    continue;
                }
                let mut tree = TreeUi::new(theme);
                let (output, _, panel) = tree.frame(vec![]);
                let text = text_rect(&output, label);
                let pos = match target {
                    "text" => text.center(),
                    "indent" => egui::pos2(panel.left() + 3.0, text.center().y),
                    "empty right" => egui::pos2(panel.right() - 50.0, text.center().y),
                    // Leave the panel's resize grip in control at the border.
                    "right edge" => egui::pos2(panel.right() - 10.0, text.center().y),
                    "size" => text_rect(&output, "12 B").center(),
                    _ => unreachable!(),
                };
                if target != "text" {
                    assert!(!text.contains(pos), "{target}");
                }
                let Some(TreeAction::OpenFile {
                    path,
                    label: opened_label,
                    read_only: opened_read_only,
                }) = tree.click(pos)
                else {
                    panic!("no file action: {theme:?}, {label}, {target}");
                };
                assert_eq!(path, tree.root.join(relative));
                assert_eq!(opened_label, label);
                assert_eq!(opened_read_only, read_only);
            }
        }
    }
}

#[test]
fn hovering_empty_row_space_highlights_the_full_row_and_shows_a_relative_path() {
    for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
        for (label, tooltip) in [
            ("rtl/top.v", "rtl/top.v"),
            ("yosys.log", "build/yosys.log\nOpen log (read-only)"),
        ] {
            let mut tree = TreeUi::new(theme);
            let (output, _, panel) = tree.frame(vec![]);
            let text = text_rect(&output, label);
            let target = egui::pos2(panel.right() - 50.0, text.center().y);
            let mut output = output;
            for _ in 0..4 {
                (output, _, _) = tree.frame(vec![egui::Event::PointerMoved(target)]);
            }
            assert!(
                output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Rect(rect)
                        if rect.rect.height() < 40.0
                            && rect.rect.contains(egui::pos2(panel.left() + 3.0, target.y))
                            && rect.rect.contains(egui::pos2(panel.right() - 3.0, target.y))
                            && rect.fill != Color32::TRANSPARENT)
                }),
                "missing full row hover: {theme:?}, {label}"
            );
            let copies = output
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == tooltip)
                })
                .count();
            assert_eq!(copies, if label == tooltip { 2 } else { 1 });
        }
    }
}

#[test]
fn output_paths_are_project_relative_including_external_build_directories() {
    let root = Path::new("/workspace/project");
    assert_eq!(
        project_relative_path(root, &root.join("build/yosys.log")),
        Path::new("build/yosys.log")
    );
    assert_eq!(
        project_relative_path(root, Path::new("/workspace/artifacts/yosys.log")),
        Path::new("../artifacts/yosys.log")
    );
}

#[test]
fn passive_output_labels_still_support_double_click_selection_and_copy() {
    let mut tree = TreeUi::new(egui::ThemePreference::Dark);
    let (output, _, _) = tree.frame(vec![]);
    let text = text_rect(&output, "top.config");
    for _ in 0..2 {
        assert!(tree.click(text.center()).is_none());
    }
    let (output, action, _) = tree.frame(vec![egui::Event::Copy]);
    assert!(action.is_none());
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if !text.is_empty())
    }));
}

#[test]
fn noninteractive_outputs_stay_noninteractive_and_header_arrows_still_toggle() {
    let mut tree = TreeUi::new(egui::ThemePreference::Dark);
    for label in ["top.config", "nextpnr.log"] {
        let (output, _, panel) = tree.frame(vec![]);
        let text = text_rect(&output, label);
        for pos in [
            text.center(),
            egui::pos2(panel.right() - 50.0, text.center().y),
        ] {
            assert!(tree.click(pos).is_none(), "{label}");
        }
    }
    let (output, _, _) = tree.frame(vec![]);
    let header = text_rect(&output, "Verilog (1)");
    let arrow = egui::pos2(header.left() - 9.0, header.center().y);
    assert!(tree.click(arrow).is_none());
    let (output, _, _) = tree.frame(vec![]);
    assert!(!output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "rtl/top.v")
    }));
    assert!(tree.click(arrow).is_none());
    let (output, _, _) = tree.frame(vec![]);
    text_rect(&output, "rtl/top.v");
}

#[test]
fn build_step_rows_explain_listed_images_without_adding_an_empty_swap_step() {
    for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
        for source in ["none", "manifest", "ecpbram.txt", "empty override"] {
            let mut tree = TreeUi::new(theme);
            fs::write(tree.root.join("fw0.hex"), "01\n").unwrap();
            fs::write(tree.root.join("fw1.hex"), "02\n").unwrap();
            let manifest = tree.root.join("ecp5.toml");
            let mut contents = fs::read_to_string(&manifest).unwrap();
            match source {
                "manifest" => contents.push_str("\n[memory]\nimages=['fw0.hex', 'fw1.hex']\n"),
                "ecpbram.txt" | "empty override" => {
                    fs::write(tree.root.join("ecpbram.txt"), "fw0.hex\nfw1.hex\n").unwrap();
                    if source == "empty override" {
                        contents.push_str("\n[memory]\nimages=[]\n");
                    }
                }
                _ => {}
            }
            fs::write(manifest, contents).unwrap();
            tree.project = Ok(Project::open(&tree.root).unwrap());
            let listed = matches!(source, "manifest" | "ecpbram.txt");
            let steps = default_steps(tree.project.as_ref().unwrap());
            let expected = if listed {
                vec![
                    Step::Synthesis,
                    Step::PlaceRoute,
                    Step::Swap,
                    Step::Bitstream,
                ]
            } else {
                vec![Step::Synthesis, Step::PlaceRoute, Step::Bitstream]
            };
            assert_eq!(
                steps.iter().map(|(step, _)| *step).collect::<Vec<_>>(),
                expected
            );
            assert!(steps
                .iter()
                .all(|(_, state)| matches!(state, StepProgress::Pending)));
            let (output, _, _) = tree.frame(vec![]);
            let count = |needle: &str| {
                output.shapes.iter().filter(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == needle)
                }).count()
            };
            assert_eq!(count(NO_MEMORY_IMAGES), usize::from(!listed));
            assert_eq!(count("Memory images (ecpbram)"), usize::from(listed));
            assert_eq!(count("○"), steps.len());
            let target = text_rect(
                &output,
                if listed {
                    "Memory images (ecpbram)"
                } else {
                    NO_MEMORY_IMAGES
                },
            )
            .center();
            let mut output = output;
            for _ in 0..4 {
                (output, _, _) = tree.frame(vec![egui::Event::PointerMoved(target)]);
            }
            let tooltip = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().contains(MEMORY_IMAGES_RULE) => {
                        Some(text.galley.text())
                    }
                    _ => None,
                })
                .expect("the memory row explains the selection rule");
            assert_eq!(tooltip.contains("Swaps:\nfw0.hex\nfw1.hex"), listed);
        }
    }
}

#[test]
fn build_step_names_and_statuses_stay_inside_their_rows() {
    for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
        // project.rs uses 260 points; egui's left panel minimum is 96.
        for width in [260.0, 96.0] {
            for step in [
                Step::Synthesis,
                Step::PlaceRoute,
                Step::Swap,
                Step::Bitstream,
            ] {
                for state in [
                    StepProgress::Pending,
                    StepProgress::Running {
                        started: Instant::now(),
                    },
                    StepProgress::Done { seconds: 0.4 },
                    StepProgress::Skipped,
                    StepProgress::Failed { seconds: Some(0.4) },
                    StepProgress::Failed { seconds: None },
                    StepProgress::NotRun,
                ] {
                    let ctx = egui::Context::default();
                    crate::theme::install(&ctx, theme);
                    let mut rects = None;
                    let output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1500.0, 898.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            let panel = egui::Panel::left("project_tree")
                                .resizable(true)
                                .default_size(width)
                                .show(ui, |ui| {
                                    egui::ScrollArea::vertical()
                                        .id_salt("project_tree_scroll")
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            let available = ui.available_rect_before_wrap();
                                            let height = ui.spacing().interact_size.y;
                                            let row = step_row(ui, step, &state, None);
                                            let (name, status) = row.inner;
                                            let row = row.response.rect;
                                            let context =
                                                format!("{theme:?}, {width}, {step}, {state:?}");
                                            assert_eq!(row.height(), height, "{context}");
                                            assert_eq!(row.left(), available.left(), "{context}");
                                            assert_eq!(row.right(), available.right(), "{context}");
                                            assert!(row.contains_rect(name.rect), "{context}");
                                            if let Some(status) = &status {
                                                assert!(
                                                    row.contains_rect(status.rect),
                                                    "{context}"
                                                );
                                                assert!(
                                                    !name.rect.intersects(status.rect),
                                                    "{context}: {:?} overlaps {:?}",
                                                    name.rect,
                                                    status.rect
                                                );
                                                assert_eq!(status.rect.right(), row.right());
                                            }
                                            let next = ui.label("following row");
                                            assert_eq!(
                                                next.rect.top(),
                                                row.bottom() + ui.spacing().item_spacing.y,
                                                "{context}"
                                            );
                                            rects = Some((row, name.rect, status.map(|s| s.rect)));
                                        });
                                });
                            assert_eq!(panel.response.rect.width(), width);
                        },
                    );
                    let (row, name, status) = rects.unwrap();
                    let texts: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::epaint::Shape::Text(text) if row.contains(text.pos) => Some(text),
                            _ => None,
                        })
                        .collect();
                    let name_text = texts
                        .iter()
                        .find(|text| text.galley.text() == step.to_string())
                        .unwrap();
                    // Check the painted text as well as the widget responses.
                    for text in &texts {
                        let painted = text.galley.rect.translate(text.pos.to_vec2());
                        assert!(row.contains_rect(painted), "{step}, {state:?}: {painted:?}");
                        if text.galley.text() == step.to_string() {
                            assert!(name.contains_rect(painted));
                        } else if let Some(status) = status {
                            assert!(!name.intersects(painted));
                            assert!(!status.intersects(painted) || status.contains_rect(painted));
                        }
                        assert_eq!(text.galley.rows.len(), 1);
                    }
                    if width == 96.0
                        || (step == Step::PlaceRoute && matches!(state, StepProgress::Skipped))
                    {
                        assert!(name_text.galley.elided, "{step}, {state:?}, {width}");
                        assert_eq!(name_text.galley.rows[0].glyphs.last().unwrap().chr, '…');
                    }
                    let icon_text = usize::from(!matches!(state, StepProgress::Running { .. }));
                    assert_eq!(texts.len(), 1 + icon_text + usize::from(status.is_some()));
                }
            }
        }
    }
}

#[test]
fn hovering_the_name_or_status_shows_both_in_full() {
    for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
        for width in [260.0, 96.0] {
            for hover_status in [false, true] {
                let ctx = egui::Context::default();
                crate::theme::install(&ctx, theme);
                ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
                let mut target = None;
                let mut output = egui::FullOutput::default();
                for frame in 0..4 {
                    output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1500.0, 898.0),
                            )),
                            time: Some(frame as f64),
                            events: target.map(egui::Event::PointerMoved).into_iter().collect(),
                            ..Default::default()
                        },
                        |ui| {
                            egui::Panel::left("project_tree")
                                .resizable(true)
                                .default_size(width)
                                .show(ui, |ui| {
                                    let row = step_row(
                                        ui,
                                        Step::PlaceRoute,
                                        &StepProgress::Skipped,
                                        None,
                                    );
                                    let (name, status) = row.inner;
                                    target = Some(if hover_status {
                                        status.unwrap().rect.center()
                                    } else {
                                        name.rect.center()
                                    });
                                });
                        },
                    );
                }
                assert!(
                    output.shapes.iter().any(|shape| {
                        matches!(&shape.shape, egui::epaint::Shape::Text(text)
                        if text.galley.text()
                            == "Place and route (nextpnr-ecp5)\ninputs unchanged"
                            && !text.galley.elided)
                    }),
                    "{theme:?}, {width}, status={hover_status}"
                );
            }
        }
    }
}

#[test]
fn root_and_source_group_context_menus_request_new_files_and_disable_when_busy() {
    for (label, expected) in [
        ("top (LFE5U-85F/CABGA381)", "rtl/counter.v"),
        ("Verilog (1)", "rtl/counter.v"),
        ("Constraints", "constraints.lpf"),
        ("Files (1)", "data.hex"),
        ("Memory images (1)", "data.hex"),
    ] {
        for busy in [false, true] {
            let mut tree = TreeUi::new(egui::ThemePreference::Dark);
            std::fs::write(tree.root.join("data.hex"), "00").unwrap();
            std::fs::write(tree.root.join("image.hex"), "00").unwrap();
            std::fs::write(tree.root.join("ecp5.toml"), "[sources]\nverilog = ['rtl/top.v']\nfiles = ['data.hex']\n[memory]\nimages = ['image.hex']\n").unwrap();
            tree.project = Ok(Project::open(&tree.root).unwrap());
            tree.busy = busy;
            let (drawn, _, _) = tree.frame(vec![]);
            let pos = text_rect(&drawn, label).center();
            for pressed in [true, false] {
                tree.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            let (menu, _, _) = tree.frame(vec![]);
            let menu_pos = text_rect(&menu, "New file…").center();
            let action = tree.click(menu_pos);
            if busy {
                assert!(action.is_none());
            } else {
                assert!(
                    matches!(action, Some(TreeAction::NewFile(path)) if path == Path::new(expected)),
                    "{label}"
                );
            }
        }
    }
}

#[test]
fn memory_image_context_menu_offers_each_role_and_disables_edits_while_busy() {
    for extension in ["hex", "mem", "init"] {
        for is_image in [false, true] {
            for busy in [false, true] {
                let mut tree = TreeUi::new(egui::ThemePreference::Dark);
                let file = format!("fw.{extension}");
                fs::write(tree.root.join(&file), "00").unwrap();
                if is_image {
                    fs::write(tree.root.join("ecpbram.txt"), format!("{file}\n")).unwrap();
                }
                tree.project = Ok(Project::open(&tree.root).unwrap());
                tree.busy = busy;
                tree.frame(vec![]);
                let (output, _, _) = tree.frame(vec![]);
                let pos = text_rect(&output, &file).center();
                for pressed in [true, false] {
                    tree.frame(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                }
                tree.frame(vec![]);
                let (output, _, _) = tree.frame(vec![]);
                let label = if is_image {
                    "Stop using as BRAM image"
                } else {
                    "Use as BRAM image (ecpbram)…"
                };
                let pos = text_rect(&output, label).center();
                let action = tree.click(pos);
                if busy {
                    assert!(action.is_none());
                } else {
                    assert!(
                        matches!(action, Some(TreeAction::MemoryImage { relative, use_as_image }) if relative == Path::new(&file) && use_as_image != is_image)
                    );
                }
            }
        }
    }
}
