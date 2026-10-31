package service

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"sync"

	"craftwork/internal/events"
	"craftwork/internal/id"
	"craftwork/internal/policy"

	"github.com/creack/pty"
)

// TerminalView 是终端会话的展示结构。
type TerminalView struct {
	ID      string `json:"id"`
	Name    string `json:"name"`
	CWD     string `json:"cwd"`
	Running bool   `json:"running"`
}

type terminalSession struct {
	view TerminalView
	cmd  *exec.Cmd
	file *os.File
}

// TerminalService 管理用户终端（PTY）。用户终端不经过 Agent 审批，
// 与 Agent 的 shell_exec 是两条独立路径。
type TerminalService struct {
	mu        sync.Mutex
	sessions  map[string]*terminalSession
	bus       *events.Bus
	workspace func() string
}

// NewTerminalService 创建终端服务。
func NewTerminalService(bus *events.Bus, workspace func() string) *TerminalService {
	return &TerminalService{sessions: map[string]*terminalSession{}, bus: bus, workspace: workspace}
}

// List 返回所有终端会话。
func (s *TerminalService) List() []TerminalView {
	s.mu.Lock()
	defer s.mu.Unlock()
	result := make([]TerminalView, 0, len(s.sessions))
	for _, session := range s.sessions {
		result = append(result, session.view)
	}
	return result
}

// Create 新建一个终端会话。
func (s *TerminalService) Create(name, cwd string) (TerminalView, error) {
	workspace := s.workspace()
	if workspace == "" {
		return TerminalView{}, errors.New("请先选择工作区")
	}
	resolved, err := policy.Resolve(workspace, cwd)
	if err != nil {
		return TerminalView{}, err
	}
	if name == "" {
		name = "终端"
	}

	shell := os.Getenv("SHELL")
	if shell == "" {
		shell = "/bin/zsh"
	}
	command := exec.Command(shell, "-l")
	command.Dir = resolved
	command.Env = os.Environ()

	file, err := pty.Start(command)
	if err != nil {
		return TerminalView{}, fmt.Errorf("启动终端: %w", err)
	}
	_ = pty.Setsize(file, &pty.Winsize{Rows: 24, Cols: 100})

	session := &terminalSession{
		view: TerminalView{ID: id.New("term"), Name: name, CWD: resolved, Running: true},
		cmd:  command,
		file: file,
	}
	s.mu.Lock()
	s.sessions[session.view.ID] = session
	s.mu.Unlock()

	go s.readLoop(session)
	return session.view, nil
}

// Write 向终端写入用户输入。
func (s *TerminalService) Write(sessionID, data string) error {
	session := s.get(sessionID)
	if session == nil {
		return errors.New("终端不存在")
	}
	_, err := session.file.WriteString(data)
	return err
}

// Resize 同步终端尺寸。
func (s *TerminalService) Resize(sessionID string, cols, rows int) error {
	session := s.get(sessionID)
	if session == nil {
		return errors.New("终端不存在")
	}
	if cols <= 0 || rows <= 0 {
		return errors.New("尺寸无效")
	}
	return pty.Setsize(session.file, &pty.Winsize{Cols: uint16(cols), Rows: uint16(rows)})
}

// Close 关闭终端并释放进程组。
func (s *TerminalService) Close(sessionID string) error {
	s.mu.Lock()
	session := s.sessions[sessionID]
	delete(s.sessions, sessionID)
	s.mu.Unlock()
	if session == nil {
		return errors.New("终端不存在")
	}
	if session.cmd.Process != nil {
		_ = session.cmd.Process.Kill()
	}
	_ = session.file.Close()
	return nil
}

func (s *TerminalService) get(sessionID string) *terminalSession {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.sessions[sessionID]
}

func (s *TerminalService) readLoop(session *terminalSession) {
	buffer := make([]byte, 4096)
	threadID := "terminal:" + session.view.ID
	for {
		count, err := session.file.Read(buffer)
		if count > 0 {
			s.bus.Publish(events.TypeTerminalOutput, threadID, "", events.TerminalDelta{
				SessionID: session.view.ID,
				Data:      string(buffer[:count]),
			})
		}
		if err != nil {
			break
		}
	}

	exitCode := -1
	if session.cmd.ProcessState != nil {
		exitCode = session.cmd.ProcessState.ExitCode()
	}
	s.mu.Lock()
	if current, ok := s.sessions[session.view.ID]; ok {
		current.view.Running = false
	}
	s.mu.Unlock()
	s.bus.Publish(events.TypeTerminalExit, threadID, "", map[string]any{
		"sessionId": session.view.ID,
		"exitCode":  exitCode,
	})
}
