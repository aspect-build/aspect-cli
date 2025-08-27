package plugin

import "sync"

// TODO: move to its own package

type Database struct {
	Symbols []TargetSymbol

	symbolMutex sync.Mutex
}

func (d *Database) AddSymbol(label Label, symbol Symbol) {
	d.symbolMutex.Lock()
	defer d.symbolMutex.Unlock()

	d.Symbols = append(d.Symbols, TargetSymbol{
		Symbol: symbol,
		Label:  label,
	})
}
