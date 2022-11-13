package flags

const (
	// The prefix that Bazel uses for negative flags such as --nohome_rc
	BazelNoPrefix = "no"
)

// Prefixes a flag name with "no" to determine the Bazel negative flag from a flag name.
// For example, `nohome_rc` is the negative of `home_rc` in Bazel.
func NoName(name string) string {
	return BazelNoPrefix + name
}
