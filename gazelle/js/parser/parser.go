package parser

type Parser interface {
	ParseImports(filePath, source string) ([]string, []error)
}
