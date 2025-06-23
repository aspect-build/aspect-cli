package flags

import (
	"testing"

	. "github.com/onsi/gomega"
)

func TestEmptyArgs(t *testing.T) {
	g := NewGomegaWithT(t)
	g.Expect(ParseSet(nil, nil)).To(BeEmpty())
	g.Expect(ParseSet([]string{}, []string{})).To(Equal([]string{}))
	g.Expect(ParseSet([]string{"a", "b"}, []string{})).To(Equal([]string{"a", "b"}))
}

func TestModArgs(t *testing.T) {
	g := NewGomegaWithT(t)
	g.Expect(ParseSet([]string{"b", "a"}, []string{"+b"})).To(Equal([]string{"b", "a"}))
	g.Expect(ParseSet([]string{"b", "a"}, []string{"-b,+b"})).To(Equal([]string{"b", "a"}))
	g.Expect(ParseSet([]string{"b", "a"}, []string{"-b", "+b"})).To(Equal([]string{"b", "a"}))
	g.Expect(ParseSet([]string{"b", "a"}, []string{"+c"})).To(Equal([]string{"b", "a", "c"}))
	g.Expect(ParseSet([]string{"b", "a"}, []string{"-a,-b", "+c"})).To(Equal([]string{"c"}))
}

func TestOverwriteArgs(t *testing.T) {
	g := NewGomegaWithT(t)
	g.Expect(ParseSet([]string{"b", "a"}, []string{"b"})).To(Equal([]string{"b"}))
	g.Expect(ParseSet([]string{"b", "a"}, []string{"c"})).To(Equal([]string{"c"}))
	g.Expect(ParseSet([]string{"b", "a"}, []string{"+d,+e,c"})).To(Equal([]string{"c"}))
	g.Expect(ParseSet([]string{"b", "a"}, []string{"+d,+e", "c"})).To(Equal([]string{"c"}))
}
