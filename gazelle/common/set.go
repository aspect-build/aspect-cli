package gazelle

import (
	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/emirpasic/gods/sets/treeset"
)

// A basic set of label.Labels with logging of set modifications.
type LabelSet struct {
	from   label.Label
	labels *treeset.Set
}

func NewLabelSet(from label.Label) *LabelSet {
	return &LabelSet{
		from:   from,
		labels: treeset.NewWithStringComparator(),
	}
}

func (s *LabelSet) Add(l *label.Label) {
	if s.from.Equal(*l) {
		BazelLog.Debugf("ignore %q dependency on self", s.from.String())
		return
	}

	// Convert to a relative label for simpler labels in BUILD files
	relL := l.Rel(s.from.Repo, s.from.Pkg)

	if d := relL.String(); !s.labels.Contains(d) {
		BazelLog.Debugf("add %q dependency: %q", s.from.String(), d)

		s.labels.Add(d)
	}
}

func (s *LabelSet) Empty() bool {
	return s.labels.Empty()
}

func (s *LabelSet) Labels() []string {
	labels := make([]string, 0, s.labels.Size())
	for _, l := range s.labels.Values() {
		labels = append(labels, l.(string))
	}
	return labels
}
