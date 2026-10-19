# Run from a workspace with a //lib package: the fixture used in the README (BUILD.bazel with //:all, //:hello and //lib:lib).
def _impl(ctx):
    rules = ctx.analysis().query("kind(rule, //...)")
    rule = [r for r in rules if r.label.package == "lib" and r.label.name == "lib"][0]
    label = rule.label
    srcs = rule.attr("srcs")
    pkg = ctx.analysis().package("//lib")
    ctx.output.print("result set:", type(rules), "rule:", type(rule), "label:", type(label))
    ctx.output.print("package:", type(pkg), "targets:", type(pkg.targets), "runtime:", type(ctx.java.runtime))
    ctx.output.print("attribute list:", type(srcs), "element:", type(srcs[0]), "visibility:", type(rule.visibility))
    again = [t for t in ctx.analysis().query("//lib:lib")][0].label
    ctx.output.print("labels equal:", label == again, "different:", label != srcs[0])
    ctx.output.print("label as dict key:", {label: "found"}[again])

main = buildx_main(impl = _impl, cli_args = {})
