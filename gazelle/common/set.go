package gazelle

import (
	. "aspect.build/cli/gazelle/common/log"
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
	d := l.String()
	if !s.labels.Contains(d) {
		BazelLog.Debugf("add dependency '%s' to '%s'", d, s.from.String())

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
