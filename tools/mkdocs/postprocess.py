import re

def on_page_markdown(markdown, **kwargs):
    if "query.eval" in markdown:
        markdown = markdown.replace(
            "typing.Iterable['@link@ /lib/rule @@ rule @link@'",
            "target_set['@link@ /lib/rule @@ rule @link@'"
        )
    return markdown.replace('<pre class="language-python"><code>', '```python\n').replace("</code></pre>", "\n```")


def on_page_content(html, **kwargs):
    return re.sub(
        r'<span class="s1">&#39;@link@ ([\w/]+) @@ ([\w\.]+) @link@&#39;</span>',
        '<a href="\\1" class="n" style="cursor:pointer; text-decoration-line:underline; font-weight: 700;">\\2</a>',
        html
    )
