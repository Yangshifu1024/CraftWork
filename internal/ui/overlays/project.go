// 创建项目对话框：表单 → 信任确认两视图（对齐 React 版 CreateProjectDialog
// 与 ZCode 交互）。项目名称未编辑时提交空串，由后端从主要目录名推导；
// 源文件夹走系统多选对话框（在后台 goroutine 打开，不阻塞 view）；
// 提交前弹「信任此文件夹吗？」——每次创建都确认，信任结果不持久化。
//
// Esc 的视图相关退出：信任确认回到表单，表单直接关闭；信任视图的默认
// 焦点在「取消」——信任必须是明确选择，不提供 Enter 直通的确认路径。

package overlays

import (
	"strings"

	mygoapp "github.com/egoist/mygo"
	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// projectState 是创建项目对话框的界面状态。
type projectState struct {
	// open 是对话框是否打开。
	open bool
	// trust 为真表示当前在信任确认视图。
	trust bool
	// name 是项目名称；nameEdited 记录用户是否改过（未改过提交空串）。
	name       string
	nameEdited bool
	// folders 是已选源文件夹（绝对路径），第一个是主要目录。
	folders []string
	// busy 是创建请求进行中（按钮禁用防重复提交）。
	busy bool
}

// drawProject 画创建项目对话框。
func (s *Store) drawProject(c *mygo.Context) {
	st := &s.project
	if !st.open {
		return
	}
	p := pal(c)

	panel := mygo.DialogBase(c, &st.open, func(backdrop, panel mygo.Element) {
		styleBackdrop(backdrop)
		stylePanel(c, panel)
		panel.Width(460).Padding(theme.Space8, theme.Space8, theme.Space7, theme.Space8).Gap(theme.Space6)
		if st.trust {
			// 信任确认视图。
			mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
				title := "信任此文件夹吗？"
				if len(st.folders) > 1 {
					title = "信任这些文件夹吗？"
				}
				mygo.Text(c, title).FontSize(theme.Fonts.LG()).Bold().Grow(1)
				if iconBtn(c, "x", "关闭信任确认").Clicked() {
					st.trust = false
				}
			})
			mutedText(c, "此电脑")
			for _, folder := range st.folders {
				mygo.Box(c).Fill().Padding(theme.Space4, theme.Space5).
					Radius(theme.RadiusRow).Background(p.BgSunken).
					BorderWidth(1).BorderColor(p.Border).Children(func() {
					mygo.Text(c, folder).FontSize(theme.Fonts.SM()).TextColor(p.Text2)
				})
			}
			mygo.Text(c, "CraftWork 可在当前权限设置允许的范围内读取、编辑这些文件夹中的文件，并运行命令。请仅在你信任列出的所有文件夹时继续。").
				FontSize(theme.Fonts.SM()).TextColor(p.Text3)
			actionsRow(c, func() {
				cancel := mygo.Button(c, "取消").AutoFocus()
				if cancel.Clicked() {
					st.trust = false
				}
				label := "信任文件夹并创建"
				if st.busy {
					label = "创建中…"
				}
				if primaryBtn(c, label).Disabled(st.busy).Clicked() {
					s.createProject(c, st)
				}
			})
		} else {
			// 表单视图。
			mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).Children(func() {
				mygo.Text(c, "创建项目").FontSize(theme.Fonts.LG()).Bold().Grow(1)
				if iconBtn(c, "x", "关闭创建项目").Clicked() {
					st.open = false
				}
			})
			mygo.Field(c, "项目名称", func() {
				mygo.TextInput(c, &st.name).Fill().Height(theme.HeightInput).
					Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
					Placeholder("新项目").AutoFocus()
			}).Fill()
			// 设备是纯展示（本地部署只有此电脑）。
			mygo.Field(c, "设备", func() {
				mygo.Row(c).Gap(theme.Space3).AlignItems(mygo.Center).Children(func() {
					icons.Icon(c, "monitor").Size(14, 14).TextColor(p.Text3)
					mygo.Text(c, "此电脑").TextColor(p.Text2)
				})
			})
			s.projectFolders(c, st)

			actionsRow(c, func() {
				if mygo.Button(c, "取消").Clicked() {
					st.open = false
				}
				if primaryBtn(c, "创建项目").Disabled(len(st.folders) == 0).Clicked() {
					st.trust = true
				}
			})
		}
	})
	if panel.Dismissed() {
		// Esc / 遮罩：信任态回表单（保持对话框打开），表单态直接关闭。
		if st.trust {
			st.trust = false
			st.open = true
		} else {
			st.open = false
		}
	}
}

// projectFolders 画源文件夹区：空态大按钮或已选列表 + 追加行。
func (s *Store) projectFolders(c *mygo.Context, st *projectState) {
	p := pal(c)
	mygo.Field(c, "源文件夹", func() {
		mygo.Column(c).Fill().Gap(theme.Space2).Children(func() {
			if len(st.folders) == 0 {
				drop := mygo.ButtonBase(c)
				drop.Fill().Height(88).Radius(theme.RadiusCard).Center().
					Border(1, p.BorderStrong).Background(p.BgInput)
				drop.Children(func() {
					mygo.Column(c).Center().Gap(theme.Space2).Children(func() {
						icons.Icon(c, "folder-plus").Size(18, 18).TextColor(p.Text3)
						mygo.Text(c, "选择文件夹")
						mygo.Text(c, "CraftWork 可读取和编辑这些文件夹").FontSize(theme.Fonts.SM()).TextColor(p.Text4)
					})
				})
				if drop.Clicked() {
					s.pickProjectFolders(st)
				}
				return
			}
			for i, folder := range st.folders {
				folder, index := folder, i
				mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space4).
					Padding(theme.Space3, theme.Space4).Radius(theme.RadiusRow).
					Background(p.BgInput).Children(func() {
					icons.Icon(c, "folder").Size(14, 14).TextColor(p.Text3)
					mygo.Text(c, pathLeaf(folder)).Grow(1).SingleLine()
					if index == 0 {
						badge(c, "主要", p.Accent, p.AccentSoft)
					}
					if iconBtn(c, "x", "移除文件夹 "+pathLeaf(folder)).Clicked() {
						st.folders = append(st.folders[:index], st.folders[index+1:]...)
					}
				})
			}
			add := mygo.ButtonBase(c)
			add.Fill().Height(32).Radius(theme.RadiusRow).AlignItems(mygo.Center).Gap(theme.Space4).
				Padding(0, theme.Space4).Background(p.BgSunken)
			add.Children(func() {
				icons.Icon(c, "folder-plus").Size(14, 14).TextColor(p.Text3)
				mygo.Text(c, "添加文件夹").TextColor(p.Text2)
			})
			if add.Clicked() {
				s.pickProjectFolders(st)
			}
		})
	}).Fill()
}

// pickProjectFolders 弹系统多选目录对话框（必须在 goroutine 里调用原生
// 对话框，避免 view 等待），去重后追加到列表。
func (s *Store) pickProjectFolders(st *projectState) {
	s.runAsync(func() func() {
		paths, err := mygoapp.Dialog.Open(mygoapp.OpenDialogOptions{
			Title:             "选择源文件夹",
			Directory:         true,
			Multiple:          true,
			CreateDirectories: true,
			ButtonLabel:       "添加文件夹",
		})
		return func() {
			if err != nil {
				Notify("选择文件夹失败", err.Error(), ToneDanger)
				return
			}
			for _, path := range paths {
				dup := false
				for _, existing := range st.folders {
					if existing == path {
						dup = true
						break
					}
				}
				if !dup {
					st.folders = append(st.folders, path)
				}
			}
		}
	})
}

// createProject 以可信档创建项目（AddProject 对主要目录做解析与重名处理）。
func (s *Store) createProject(c *mygo.Context, st *projectState) {
	if st.busy || len(st.folders) == 0 {
		return
	}
	st.busy = true
	name := ""
	if st.nameEdited {
		name = strings.TrimSpace(st.name)
	}
	folders := append([]string(nil), st.folders...)
	if s.be == nil || s.be.Workspace == nil {
		st.busy = false
		return
	}
	s.runAsync(func() func() {
		view, err := s.be.Workspace.AddProject(name, folders, "trusted")
		return func() {
			st.busy = false
			if err != nil {
				Notify("创建项目失败", err.Error(), ToneDanger)
				st.trust = false
				return
			}
			st.open = false
			st.trust = false
			st.name = ""
			st.nameEdited = false
			st.folders = nil
			label := view.Name
			if label == "" {
				label = pathLeaf(folders[0])
			}
			Notify("已创建项目", label, ToneOK)
			// 侧栏刷新与新建会话由 04 号票的编排钩子完成；钩子未接线时
			// 只提示成功（工作区列表下次打开侧栏时会重新读取）。
			if s.hooks.ProjectCreated != nil {
				s.hooks.ProjectCreated(view.ID)
			}
		}
	})
}
