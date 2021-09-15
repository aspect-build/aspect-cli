# Target pattern syntax

The BUILD file label syntax is used to specify a single target. Target patterns
generalize this syntax to sets of targets, and also support
working-directory-relative forms, recursion, subtraction and filtering.
Examples:

Specifying a single target:

//foo/bar:wiz The single target '//foo/bar:wiz'. foo/bar/wiz Equivalent to:
'//foo/bar/wiz:wiz' if foo/bar/wiz is a package, '//foo/bar:wiz' if foo/bar is a
package, '//foo:bar/wiz' otherwise. //foo/bar Equivalent to '//foo/bar:bar'.

Specifying all rules in a package:

//foo/bar:all Matches all rules in package 'foo/bar'.

Specifying all rules recursively beneath a package:

//foo/...:all Matches all rules in all packages beneath directory 'foo'.
//foo/... (ditto)

By default, directory symlinks are followed when performing this recursive
traversal, except those that point to under the output base (for example, the
convenience symlinks that are created in the root directory of the workspace)
But we understand that your workspace may intentionally contain directories with
weird symlink structures that you don't want consumed. As such, if a directory
has a file named
'DONT_FOLLOW_SYMLINKS_WHEN_TRAVERSING_THIS_DIRECTORY_VIA_A_RECURSIVE_TARGET_PATTERN'
then symlinks in that directory won't be followed when evaluating recursive
target patterns.

Working-directory relative forms: (assume cwd = 'workspace/foo')

Target patterns which do not begin with '//' are taken relative to the working
directory. Patterns which begin with '//' are always absolute.

...:all Equivalent to '//foo/...:all'. ... (ditto)

bar/...:all Equivalent to '//foo/bar/...:all'. bar/... (ditto)

bar:wiz Equivalent to '//foo/bar:wiz'. :foo Equivalent to '//foo:foo'.

bar Equivalent to '//foo/bar:bar'. foo/bar Equivalent to '//foo/foo/bar:bar'.

bar:all Equivalent to '//foo/bar:all'. :all Equivalent to '//foo:all'.

Summary of target wildcards:

:all, Match all rules in the specified packages. :\*, :all-targets Match all
targets (rules and files) in the specified packages, including .par and
\_deploy.jar files.

Subtractive patterns:

Target patterns may be preceded by '-', meaning they should be subtracted from
the set of targets accumulated by preceding patterns. (Note that this means
order matters.) For example:

    % bazel build -- foo/... -foo/contrib/...

builds everything in 'foo', except 'contrib'. In case a target not under
'contrib' depends on something under 'contrib' though, in order to build the
former bazel has to build the latter too. As usual, the '--' is required to
prevent '-f' from being interpreted as an option.

When running the test command, test suite expansion is applied to each target
pattern in sequence as the set of targets is evaluated. This means that
individual tests from a test suite can be excluded by a later target pattern. It
also means that an exclusion target pattern which matches a test suite will
exclude all tests which that test suite references. (Targets that would be
matched by the list of target patterns without any test suite expansion are also
built unless --build_tests_only is set.)
