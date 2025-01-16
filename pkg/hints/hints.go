package hints

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"regexp"
	"strings"
	"sync"

	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/creack/pty"
	"golang.org/x/term"
)

type Hints struct {
	hintMap                map[*regexp.Regexp]string
	hints                  *hintSet
	hintsMutex             sync.Mutex
	wg                     sync.WaitGroup
	originalStdout         *os.File
	originalStderr         *os.File
	originalDefaultStreams *ioutils.Streams
	stdoutR                *os.File
	stdoutW                *os.File
	stderrR                *os.File
	stderrW                *os.File
}

func New() *Hints {
	return &Hints{
		hintMap: map[*regexp.Regexp]string{},
		hints:   &hintSet{nodes: make(map[hintNode]struct{})},
	}
}

func (h *Hints) Configure(data interface{}) error {
	config, err := unmarshalInterfaceToMap(data)
	if err != nil {
		return err
	}

	for pattern, msg := range config {
		regex, err := regexp.Compile(pattern)
		if err != nil {
			return err
		}
		h.hintMap[regex] = msg
	}

	return nil
}

func (h *Hints) Attach() error {
	if len(h.hintMap) == 0 {
		return nil
	}

	// Create stream pipes using pty if Stdout/Stderr is a terminal other using a standard pipe
	var err error
	stdoutTerm := term.IsTerminal(int(os.Stdout.Fd()))
	stderrTerm := term.IsTerminal(int(os.Stderr.Fd()))
	if stdoutTerm {
		h.stdoutR, h.stdoutW, err = pty.Open()
	} else {
		h.stdoutR, h.stdoutW, err = os.Pipe()
	}
	if err != nil {
		return err
	}
	if stderrTerm {
		h.stderrR, h.stderrW, err = pty.Open()
	} else {
		h.stderrR, h.stderrW, err = os.Pipe()
	}
	if err != nil {
		return err
	}

	// Save original Stdout, Stderr and DefaultStreams and override
	h.originalStdout = os.Stdout
	h.originalStderr = os.Stderr
	h.originalDefaultStreams = &ioutils.DefaultStreams
	os.Stdout = h.stdoutW
	os.Stderr = h.stderrW
	ioutils.DefaultStreams = ioutils.Streams{
		Stdin:  ioutils.DefaultStreams.Stdin,
		Stdout: h.stdoutW,
		Stderr: h.stderrW,
	}

	// Create goroutines to forward streams and create hints on regex matches
	h.wg.Add(2)

	go func() {
		defer h.wg.Done()
		reader := bufio.NewReader(h.stdoutR)
		for {
			line, err := reader.ReadString('\n')
			if err != nil {
				if err != io.EOF {
					fmt.Fprintf(h.originalStderr, "Error reading from stdout: %v\n", err)
				}
				break
			}
			h.ProcessLine(line)
			fmt.Fprint(h.originalStdout, line)
		}
	}()

	go func() {
		defer h.wg.Done()
		reader := bufio.NewReader(h.stderrR)
		for {
			line, err := reader.ReadString('\n')
			if err != nil {
				if err != io.EOF {
					fmt.Fprintf(h.originalStderr, "Error reading from stderr: %v\n", err)
				}
				break
			}
			h.ProcessLine(line)
			fmt.Fprint(h.originalStderr, line)
		}
	}()

	return nil
}

func (h *Hints) Detach() {
	if h.stdoutW != nil {
		h.stdoutW.Close()
		h.stdoutW = nil
	}
	if h.stderrW != nil {
		h.stderrW.Close()
		h.stderrW = nil
	}
	h.wg.Wait()
	if h.originalStdout != nil {
		os.Stdout = h.originalStdout
		h.originalStdout = nil
	}
	if h.originalStderr != nil {
		os.Stderr = h.originalStderr
		h.originalStderr = nil
	}
	if h.originalDefaultStreams != nil {
		ioutils.DefaultStreams = *h.originalDefaultStreams
		h.originalDefaultStreams = nil
	}
}

func (h *Hints) ProcessLine(line string) {
	for regex, hint := range h.hintMap {
		matches := regex.FindStringSubmatch(line)
		if len(matches) > 0 {
			// apply regex capture group replacements to given hint
			for i, match := range matches {
				if i == 0 {
					// skipping the first match because it will always contain the entire result
					// of the regex match. We are only after specific capture groups
					continue
				}
				hint = strings.ReplaceAll(hint, fmt.Sprint("$", i), match)
			}
			h.hintsMutex.Lock()
			h.hints.insert(hint)
			h.hintsMutex.Unlock()
		}
	}
}

func (h *Hints) PrintHints(f *os.File) {
	if h.hints.size == 0 {
		return
	}
	printBreak(f)
	printMiddle(f, "[Aspect CLI]")
	printMiddle(f, "")
	for node := h.hints.head; node != nil; node = node.next {
		printMiddle(f, "- "+node.hint)
	}
	printBreak(f)
}

func printBreak(f *os.File) {
	// using buffer so that we can easily determine the current length of the string and
	// ensure we create a proper square with a straight border
	var b strings.Builder

	fmt.Fprint(&b, " ")

	for i := 0; i < 90; i++ {
		fmt.Fprint(&b, "-")
	}

	fmt.Fprint(&b, " ")

	fmt.Fprintln(f, b.String())
}

func printMiddle(f *os.File, str string) {
	// using buffer so that we can easily determine the current length of the string and
	// ensure we create a proper square with a straight border
	var b strings.Builder

	fmt.Fprint(&b, "| ")
	fmt.Fprint(&b, str)

	for b.Len() < 91 {
		fmt.Fprint(&b, " ")
	}

	fmt.Fprint(&b, "|")
	fmt.Fprintln(f, b.String())
}

func unmarshalInterfaceToMap(data interface{}) (map[string]string, error) {
	// Create a map to hold the result
	result := make(map[string]string)

	// Accept an undefined entry
	if data == nil {
		return result, nil
	}

	// Check if the input is a map
	mapData, ok := data.(map[string]interface{})
	if !ok {
		return nil, errors.New("hints config is not a map[string]interface{}")
	}

	// Convert each value to a string and populate the result map
	for key, value := range mapData {
		switch v := value.(type) {
		case string:
			result[key] = v
		case fmt.Stringer: // For types implementing the Stringer interface
			result[key] = v.String()
		default:
			// Use JSON marshaling for complex types
			jsonValue, err := json.Marshal(v)
			if err != nil {
				return nil, fmt.Errorf("failed to marshal value for hints key '%s': %w", key, err)
			}
			result[key] = string(jsonValue)
		}
	}

	return result, nil
}

type hintSet struct {
	head  *hintNode
	tail  *hintNode
	nodes map[hintNode]struct{}
	size  int
}

func (s *hintSet) insert(hint string) {
	node := hintNode{
		hint: hint,
	}
	if _, exists := s.nodes[node]; !exists {
		s.nodes[node] = struct{}{}
		if s.head == nil {
			s.head = &node
		} else {
			s.tail.next = &node
		}
		s.tail = &node
		s.size++
	}
}

type hintNode struct {
	next *hintNode
	hint string
}
