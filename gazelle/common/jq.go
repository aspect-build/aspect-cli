package gazelle

import (
	"sync"

	"github.com/itchyny/gojq"
)

var jqQueryCache = sync.Map{}

func ParseJsonQuery(query string) (*gojq.Code, error) {
	q, loaded := jqQueryCache.Load(query)
	if !loaded {
		p, err := gojq.Parse(query)
		if err != nil {
			return nil, err
		}
		q, err = gojq.Compile(p)
		if err != nil {
			return nil, err
		}
		q, _ = jqQueryCache.LoadOrStore(query, q)
	}

	return q.(*gojq.Code), nil
}
