// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at
// rev 1af8299c291f24fca68dfb6b7ef1b5c0475892e1. BSD-3-Clause; see NOTICE.md in this directory for the license
// text and the vendoring rules. Do not edit by hand.

package io.github.gedgygedgy.rust.ops;

import java.io.Closeable;

final class FnFunctionImpl<T, R> implements FnFunction<T, R> {
    private final FnAdapter<FnFunctionImpl<T, R>, T, Void, R> adapter;

    private FnFunctionImpl(FnAdapter<FnFunctionImpl<T, R>, T, Void, R> adapter) {
        this.adapter = adapter;
    }

    @Override
    public R apply(T t) {
        return this.adapter.call(this, t, null);
    }

    @Override
    public void close() {
        this.adapter.close();
    }
}
