#!/usr/bin/env bash
# Demo failure exercising the inline test-log snippet on the CI status surfaces.
# Emits a realistic, multi-line pytest-style failure so the tail-with-error-anchor
# windowing has markers to anchor on. Tagged `manual`; run explicitly via
# `aspect test //examples/test_states:demo_test_fail`.
set -u

cat <<'LOG'
============================= test session starts ==============================
platform linux -- Python 3.11.4, pytest-7.4.0, pluggy-1.2.0
collected 3 items

tests/test_widget.py::test_render PASSED                                  [ 33%]
tests/test_widget.py::test_serialize PASSED                               [ 66%]
tests/test_widget.py::test_totals FAILED                                  [100%]

=================================== FAILURES ===================================
________________________________ test_totals __________________________________

    def test_totals():
        cart = Cart(items=[Item("apple", 2), Item("pear", 3)])
>       assert cart.total() == 6
E       assert 5 == 6
E        +  where 5 = <Cart items=2>.total()

tests/test_widget.py:42: AssertionError
=========================== short test summary info ============================
FAILED tests/test_widget.py::test_totals - assert 5 == 6
========================= 1 failed, 2 passed in 0.07s ==========================
LOG

exit 1
