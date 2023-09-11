package parser

type ParseResult struct {
	Imports []string
	Modules []string
}

type Parser interface {
	ParseSource(filePath, source string) (ParseResult, []error)
}
