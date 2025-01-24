package progress

import (
	"context"
	"flag"
	"fmt"
	"math"
	"syscall"
	"time"

	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/repo"
	"github.com/bazelbuild/bazel-gazelle/resolve"
	"github.com/bazelbuild/bazel-gazelle/rule"
	"golang.org/x/term"
)

type progressPhase = string

const (
	progressPhaseWalk      progressPhase = "Walk"
	progressPhaseConfigure progressPhase = "Generate"
	progressPhaseGenerate  progressPhase = "Generate"
	progressPhaseIndex     progressPhase = "Index"
	progressPhaseResolve   progressPhase = "Resolve"
	progressPhaseWrite     progressPhase = "Update"
	progressPhaseDone      progressPhase = "\n"
)

func NewLanguage() language.Language {
	l := &progressLang{
		status: make(chan *progressStatus, 1),
	}
	go l.run(context.Background())
	return l
}

var _ config.Configurer = (*progressLang)(nil)
var _ language.Language = (*progressLang)(nil)
var _ language.LifecycleManager = (*progressLang)(nil)

type progressStatus struct {
	phase progressPhase
	what  string
	when  time.Time
}

type progressLang struct {
	status chan *progressStatus
}

func (p *progressLang) run(ctx context.Context) {
	for {
		select {
		case <-ctx.Done():
			fmt.Print("\n")
			return
		case s, ok := <-p.status:
			if !ok {
				return
			}
			writeStatus(s)
		}
	}
}

func writeStatus(s *progressStatus) {
	// NOTE(windows): syscall.Stdout is a `Handle` on windows and `int` on other platforms,
	// while `term.GetSize` only accepts the `int` type.
	var sysStdout interface{} = syscall.Stdout

	width, _, err := term.GetSize(sysStdout.(int))
	if err != nil {
		fmt.Printf("\nTerm error: %v\n", err)
		return
	}

	msgWidth := len(s.phase) + len(s.what) + 2
	extraSpace := int(math.Max(0, float64(width-msgWidth)))

	fmt.Print("\x1b7")   // save the cursor position
	fmt.Print("\x1b[2k") // erase the current line
	fmt.Printf("\x1B[33m %s %v%*s\x1B[0m", s.phase, s.what, extraSpace, " ")
	fmt.Print("\x1b8") // restore the cursor position
}

func (p *progressLang) send(phase progressPhase, what string) {
	msg := &progressStatus{
		phase: phase,
		what:  what,
		when:  time.Now(),
	}

	select {
	case p.status <- msg:
		// sent
	default:
		// dropped due to queue already being full
	}
}

func (p *progressLang) Name() string { return "progress" }

// 1. Before() all actions
func (p *progressLang) Before(ctx context.Context) {
	// Also use this gazelle-managed background context to initialize the
	// the progress goroutine.
	go p.run(ctx)

	p.send(progressPhaseWalk, "repository...")
}

// 2. Recurse into each directory
func (p *progressLang) Configure(c *config.Config, rel string, f *rule.File) {
	if rel == "" {
		p.send(progressPhaseConfigure, "...")
	}
}

// 3. Recurse out of each directory.  This os normally more expensive then
// Configure (parsing files etc) so notify per-call on this step.
func (p *progressLang) GenerateRules(args language.GenerateArgs) language.GenerateResult {
	p.send(progressPhaseGenerate, args.Rel)
	return language.GenerateResult{}
}

// 4. Done generating rules, starting indexing
func (p *progressLang) DoneGeneratingRules() {
	p.send(progressPhaseIndex, "workspace")
}

// 5. Indexing done, starting resolving
func (p *progressLang) Resolve(c *config.Config, ix *resolve.RuleIndex, rc *repo.RemoteCache, r *rule.Rule, imports interface{}, from label.Label) {
	p.send(progressPhaseResolve, "dependencies")
}

// 6. Resolving done, starting write of BUILDs to disk
func (p *progressLang) AfterResolvingDeps(ctx context.Context) {
	p.send(progressPhaseWrite, "BUILDs")
}

// Initializers
func (p *progressLang) RegisterFlags(fs *flag.FlagSet, cmd string, c *config.Config) {}
func (p *progressLang) CheckFlags(fs *flag.FlagSet, c *config.Config) error          { return nil }
func (p *progressLang) KnownDirectives() []string                                    { return nil }
func (p *progressLang) Loads() []rule.LoadInfo                                       { return nil }
func (p *progressLang) Kinds() map[string]rule.KindInfo                              { return nil }

// Per file
func (p *progressLang) Fix(c *config.Config, f *rule.File) {}

// Per rule called
func (p *progressLang) Embeds(r *rule.Rule, from label.Label) []label.Label { return nil }
func (p *progressLang) Imports(c *config.Config, r *rule.Rule, f *rule.File) []resolve.ImportSpec {
	return nil
}
