#[macro_use]
extern crate error_chain;

extern crate libc;


use std::ffi::CString;
use std::mem::transmute;
use std::path::Path;
use std::ptr::null_mut;


mod errors {
    error_chain! {
        foreign_links {
            FfiNull(::std::ffi::NulError);
        }
    }
}


#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
#[allow(dead_code)]
#[allow(improper_ctypes)]
mod crfsuite_sys {
    include!(concat!(env!("OUT_DIR"), "/crfsuite.rs"));
}


use errors::*;
use crfsuite_sys::crfsuite_create_instance_from_file;


struct Attribute {
    attr: String,
    value: f64
}

type Item = Vec<Attribute>;
type ItemSequence = Vec<Item>;

struct Tagger {
    model: crfsuite_sys::crfsuite_model_t,
    tagger: crfsuite_sys::crfsuite_tagger_t
}


impl Tagger {
    fn create_from_file<P: AsRef<Path>>(path: P) -> Result<Tagger> {
        let mut model = null_mut();

        let path_str = path.as_ref().to_str().ok_or("Path not convertible to str")?.as_bytes();

        let r = unsafe {
            crfsuite_create_instance_from_file(CString::new(path_str)?.into_raw(), &mut model)
        };

        if r != 0 {
            bail!("error while creating instance : non zero C return code...")
        }

        let model: &crfsuite_sys::crfsuite_model_t = unsafe { transmute(model) };

        let mut model = *model;

        let mut tagger = null_mut();

        if let Some(t) = model.get_tagger {
            let r = unsafe { t(&mut model, &mut tagger) };
            if r != 0 {
                bail!("error while getting tagger : non zero C return code...")
            }
        } else {
            bail!("could not retrieve tagger : no callback")
        }

        let tagger: &crfsuite_sys::crfsuite_tagger_t = unsafe { transmute(tagger) };
        let mut tagger = *tagger;

        Ok(Tagger {
            model: model,
            tagger: tagger
        })
    }

    fn create_from_memory(data: &[u8]) -> Result<Tagger> {
        unimplemented!();
    }

    fn labels(&self) -> Vec<String> {
        vec![]
    }

    fn tag(&self, input: ItemSequence) -> Vec<String> {
        vec![]
    }

    fn set(&self, input: ItemSequence) {}

    fn viterbi(&self) -> Vec<String> {
        vec![]
    }

    fn probability(&self, tags: Vec<String>) -> f64 {
        0.0
    }

    fn marginal(&self, label: &str, position: usize) -> f64 {
        0.0
    }
}

impl Drop for Tagger {
    fn drop(&mut self) {
        // TODO
        /*if let Some(r) = self.tagger.release {
            unsafe { r(&mut self.tagger) };
        }
        if let Some(r) = self.model.release {
            unsafe { r(&mut self.model) };
        }*/
    }
}


#[cfg(test)]
mod tests {
    use super::Tagger;


    #[test]
    fn it_works() {
        unsafe {
            let t = Tagger::create_from_file("/home/fredszaq/Work/crfsuite-rs-binding/model51B53Y.crfsuite");

            assert!(t.is_ok());
        }
    }
}
