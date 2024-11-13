package gazelle

import (
	"regexp"
	"sync"
)

// A cache of parsed regex strings
var regexCache = sync.Map{}

func ParseRegex(regexStr string) *regexp.Regexp {
	re, found := regexCache.Load(regexStr)
	if !found {
		re, _ = regexCache.LoadOrStore(regexStr, regexp.MustCompile(regexStr))
	}

	return re.(*regexp.Regexp)
}
