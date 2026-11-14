// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.ops;

import java.io.Closeable;

final class FnBiFunctionImpl<T, U, R> implements FnBiFunction<T, U, R> {
    private final FnAdapter<FnBiFunctionImpl<T, U, R>, T, U, R> adapter;

    private FnBiFunctionImpl(FnAdapter<FnBiFunctionImpl<T, U, R>, T, U, R> adapter) {
        this.adapter = adapter;
    }

    @Override
    public R apply(T t, U u) {
        return this.adapter.call(this, t, u);
    }

    @Override
    public void close() {
        this.adapter.close();
    }
}
