package gazelle

import (
	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/label"
	"github.com/emirpasic/gods/sets/treeset"
	"github.com/emirpasic/gods/utils"
)

// A basic set of label.Labels with logging of set modifications.
type LabelSet struct {
	from   label.Label
	labels *treeset.Set
}

func LabelComparator(a, b interface{}) int {
	return utils.StringComparator(a.(label.Label).String(), b.(label.Label).String())
}

func NewLabelSet(from label.Label) *LabelSet {
	return &LabelSet{
		from:   from,
		labels: treeset.NewWith(LabelComparator),
	}
}

func (s *LabelSet) Add(l *label.Label) {
	if s.from.Equal(*l) {
		BazelLog.Debugf("ignore %q dependency on self", s.from.String())
		return
	}

	// Convert to a relative label for simpler labels in BUILD files
	relL := l.Rel(s.from.Repo, s.from.Pkg)

	s.labels.Add(relL)
}

func (s *LabelSet) Empty() bool {
	return s.labels.Empty()
}

func (s *LabelSet) Labels() []label.Label {
	labels := make([]label.Label, 0, s.labels.Size())
	for _, l := range s.labels.Values() {
		labels = append(labels, l.(label.Label))
	}
	return labels
}
