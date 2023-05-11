use std::env;
use std::path::Path;

fn main() {
    cc::Build::new()
        .include("c/include")
        //.define("USE_SSE", "1") // TODO check if target supports SSE and enable if so
        // lbfgs
        //.file("c/lbfgs/arithmetic_ansi.h")
        //.file("c/lbfgs/arithmetic_sse_double.h")
        //.file("c/lbfgs/arithmetic_sse_float.h")
        //.file("c/include/lbfgs.h")
        .file("c/lbfgs/lbfgs.c")
        // cqdb
        .file("c/cqdb/lookup3.c")
        // .file("c/include/cqdb.h")
        .file("c/cqdb/cqdb.c")
        // crf
        .file("c/crf/dictionary.c")
        .file("c/crf/logging.c")
        //.file("c/crf/logging.h")
        .file("c/crf/params.c")
        //.file("c/crf/params.h")
        .file("c/crf/quark.c")
        //.file("c/crf/quark.h")
        .file("c/crf/rumavl.c")
        //.file("c/crf/rumavl.h")
        //.file("c/crf/vecmath.h")
        //.file("c/crf/crfsuite_internal.h")
        .file("c/crf/dataset.c")
        .file("c/crf/holdout.c")
        .file("c/crf/train_arow.c")
        .file("c/crf/train_averaged_perceptron.c")
        .file("c/crf/train_l2sgd.c")
        .file("c/crf/train_lbfgs.c")
        .file("c/crf/train_passive_aggressive.c")
        //.file("c/crf/crf1d.h")
        .file("c/crf/crf1d_context.c")
        .file("c/crf/crf1d_model.c")
        .file("c/crf/crf1d_feature.c")
        .file("c/crf/crf1d_encode.c")
        .file("c/crf/crf1d_tag.c")
        .file("c/crf/crfsuite_train.c")
        .file("c/crf/crfsuite.c")
        .flag_if_supported("-mmacosx-version-min=10.11")
        .compile("libcrfsuite.a");

    let out_dir = env::var("OUT_DIR").unwrap();

    let p = Path::new(&out_dir).join("crfsuite.rs");
    dinghy_build::dinghy_bindgen!()
        .clang_arg("-v")
        .header("c/include/crfsuite.h")
        .generate()
        .unwrap()
        .write_to_file(&p)
        .expect("Couldn't write bindings!");
}
