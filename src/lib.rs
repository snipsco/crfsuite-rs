#[macro_use]
extern crate error_chain;

extern crate libc;

use std::ffi::{CStr, CString};
use std::mem::transmute;
use std::mem::zeroed;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::slice;

mod errors {
    error_chain! {
        foreign_links {
            FfiNull(::std::ffi::NulError);
            Utf8(::std::str::Utf8Error);
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

pub struct Attribute {
    pub attr: String,
    pub value: f64
}

type Item = Vec<Attribute>;
type ItemSequence = Vec<Item>;

pub struct Tagger {
    model: crfsuite_sys::crfsuite_model_t,
    tagger: crfsuite_sys::crfsuite_tagger_t
}

impl Tagger {
    pub fn create_from_file<P: AsRef<Path>>(path: P) -> Result<Tagger> {
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

        let tagger = unsafe { *tagger };

        Ok(Tagger {
            model: model,
            tagger: tagger
        })
    }

    /*pub fn create_from_memory(data: &[u8]) -> Result<Tagger> {
        unimplemented!();
    }*/

    /*pub fn labels(&self) -> Vec<String> {
        unimplemented!();
    }*/

    pub fn tag(&mut self, input: ItemSequence) -> Result<Vec<String>> {
        &self.set(input)?;
        self.viterbi()
    }

    pub fn set(&mut self, input: ItemSequence) -> Result<()> {
        let mut attrs = null_mut();
        if let Some(g) = self.model.get_attrs {
            let r = unsafe { g(&mut self.model, &mut attrs) };
            if r != 0 {
                bail!("error while getting tagger : non zero C return code...")
            }
        } else {
            bail!("could not create attrs : no callback")
        }
        let mut attrs = unsafe { *attrs };
        let mut inst = unsafe { zeroed() };

        unsafe {
            crfsuite_sys::crfsuite_instance_init_n(&mut inst, input.len() as libc::c_int);
        }

        let mut inst_items = unsafe {
            slice::from_raw_parts_mut(inst.items, inst.num_items as usize)
        };

        for i in 0..input.len() {
            let ref item = input[i];
            let ref mut inst_item = inst_items[i];

            unsafe { crfsuite_sys::crfsuite_item_init(inst_item) };

            for i in 0..item.len() {
                let aid = if let Some(to_id) = attrs.to_id {
                    unsafe { to_id(&mut attrs, CString::new(item[i].attr.as_bytes())?.into_raw()) }
                } else {
                    bail!("could not call to_id on attr : no callback")
                };

                if 0 <= aid {
                    let cont = null_mut();
                    unsafe { crfsuite_sys::crfsuite_attribute_set(cont, aid, item[i].value) };
                    unsafe { crfsuite_sys::crfsuite_item_append_attribute(inst_item, cont) };
                }
            }
        }

        if let Some(set) = self.tagger.set {
            let r = unsafe { set(&mut self.tagger, &mut inst) };
            if r != 0 {
                unsafe { crfsuite_sys::crfsuite_instance_finish(&mut inst); }
                if let Some(release) = attrs.release {
                    unsafe { release(&mut attrs) };
                } // let's not mask the no zero return code error with this failed release...

                bail!("error while getting tagger : non zero C return code...")
            }
        } else {
            bail!("could not create attrs : no callback")
        }

        unsafe { crfsuite_sys::crfsuite_instance_finish(&mut inst); }
        if let Some(release) = attrs.release {
            unsafe { release(&mut attrs) };
        } else {
            bail!("could not release attrs : no callback...")
        }

        Ok(())
    }

    pub fn viterbi(&mut self) -> Result<Vec<String>> {
        let t: usize = if let Some(length) = self.tagger.length {
            unsafe { length(&mut self.tagger) as usize }
        } else {
            bail!("could not get tagger length : no callback")
        };

        if t <= 0 { return Ok(vec![]) }

        let mut labels = null_mut();

        if let Some(get_labels) = self.model.get_labels {
            let r = unsafe { get_labels(&mut self.model, &mut labels) };
            if r != 0 {
                bail!("failed to obtain the dictionary interface for labels")
            }
        } else {
            bail!("could not get labels : no callback")
        }

        let mut labels = unsafe { *labels };

        let mut score = 0.0;
        let mut path = vec![0; t];

        if let Some(viterbi) = self.tagger.viterbi {
            let r = unsafe { viterbi(&mut self.tagger, &mut path[0], &mut score) };
            if r != 0 {
                if let Some(release) = labels.release {
                    unsafe { release(&mut labels); }
                } // let's not mask the error with this failed release...
                bail!("failed to find the viterbi path")
            }
        }

        let mut yseq = Vec::with_capacity(t);

        for i in 0..t {
            let mut label = null();
            if let Some(to_string) = labels.to_string {
                let r = unsafe { to_string(&mut labels, path[i], &mut label) };
                if r != 0 {
                    if let Some(release) = labels.release {
                        unsafe { release(&mut labels); }
                    } // let's not mask the error with this failed release...
                    bail!("failed to convert a label identifier to string")
                }
            } else {
                bail!("could not transform to string : no callback")
            }
            yseq.push(unsafe { CStr::from_ptr(label) }.to_str()?.to_string());

            if let Some(free) = labels.free {
                unsafe { free(&mut labels, label) };
            } else {
                bail!("could not free label : no callback");
            }
        }

        if let Some(release) = labels.release {
            unsafe { release(&mut labels); }
        } else {
            bail!("could not release labels : no callback")
        }
        Ok(yseq)
    }

    /*pub fn probability(&self, tags: Vec<String>) -> f64 {
        unimplemented!();
    }*/

    /*pub fn marginal(&self, label: &str, position: usize) -> f64 {
        unimplemented!();
    }*/
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
    use super::Attribute;


    #[test]
    fn it_works() {
        let t = Tagger::create_from_file("/home/fredszaq/Work/crfsuite-rs-binding/model51B53Y.crfsuite");

        assert!(t.is_ok());

        let input = vec![
            vec![
                Attribute { attr: "is_first=1".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_1=xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1=this".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters=011010101".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[+1]=0110001111".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+2]=a".to_string(), value: 1.0 },
                Attribute { attr: "ngram_2[+1]=rare_word a".to_string(), value: 1.0 },
            ],
            vec![
                Attribute { attr: "word_cluster_brown_clusters[-1]=011010101".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2[-1]=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_1=xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_3[-1]=xxx xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-1]=this".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters=0110001111".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+1]=a".to_string(), value: 1.0 },
                Attribute { attr: "is_first[-1]=1".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[+1]=0101011111".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+2]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_2[+1]=a rare_word".to_string(), value: 1.0 },
            ],
            vec![
                Attribute { attr: "is_first[-2]=1".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-1]=0110001111".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-2]=011010101".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2[-1]=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_2[-2]=this rare_word".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_1=xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_3[-1]=xxx xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1=a".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters=0101011111".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-2]=this".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+2]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_2[+1]=rare_word rare_word".to_string(), value: 1.0 },
            ],
            vec![
                Attribute { attr: "ngram_2[-2]=rare_word a".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-1]=0101011111".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-2]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-2]=0110001111".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2[-1]=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "token_is_in_dummy_entity_1=U-".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_1=xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_3[-1]=xxx xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-1]=a".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[+1]=1111011110010".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+2]=with".to_string(), value: 1.0 },
                Attribute { attr: "ngram_2[+1]=rare_word with".to_string(), value: 1.0 },
            ],
            vec![
                Attribute { attr: "ngram_2[-2]=a rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-2]=a".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-2]=0101011111".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2[-1]=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_1=xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_3[-1]=xxx xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "token_is_in_dummy_entity_1[-1]=U-".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters=1111011110010".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+1]=with".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[+1]=1000100".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+2]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_2[+1]=with rare_word".to_string(), value: 1.0 },
            ],
            vec![
                Attribute { attr: "token_is_in_dummy_entity_1[-2]=U-".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-1]=1111011110010".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-2]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2[-1]=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_2[-2]=rare_word rare_word".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_1=xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_3[-1]=xxx xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1=with".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters=1000100".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[+1]=011010100".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+2]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_2[+1]=rare_word rare_word".to_string(), value: 1.0 },
            ],
            vec![
                Attribute { attr: "ngram_2[-2]=rare_word with".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-1]=1000100".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-2]=1111011110010".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2[-1]=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "is_last[+2]=1".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_1=xxx".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_3[-1]=xxx xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-1]=with".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters=011010100".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-2]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+2]=please".to_string(), value: 1.0 },
                Attribute { attr: "ngram_2[+1]=rare_word please".to_string(), value: 1.0 },
            ],
            vec![
                Attribute { attr: "ngram_2[-2]=with rare_word".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-1]=011010100".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[+1]=11101010110".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-2]=1000100".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_2[-1]=xxx xxx".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "is_last[+1]=1".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[+1]=please".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-2]=with".to_string(), value: 1.0 },
                Attribute { attr: "token_is_in_dummy_entity_2=U-".to_string(), value: 1.0 },
                Attribute { attr: "shape_ngram_1=xxx".to_string(), value: 1.0 },
            ],
            vec![
                Attribute { attr: "ngram_2[-2]=rare_word rare_word".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters[-2]=011010100".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-1]=rare_word".to_string(), value: 1.0 },
                Attribute { attr: "is_last=1".to_string(), value: 1.0 },
                Attribute { attr: "word_cluster_brown_clusters=11101010110".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1=please".to_string(), value: 1.0 },
                Attribute { attr: "token_is_in_dummy_entity_2[-1]=U-".to_string(), value: 1.0 },
                Attribute { attr: "ngram_1[-2]=rare_word".to_string(), value: 1.0 },
            ]
        ];

        let r = t.unwrap().tag(input);

        assert!(r.is_ok());

        assert_eq!(r.unwrap(), vec![
            "O",
            "O",
            "O",
            "B-dummy_slot_name",
            "O",
            "O",
            "O",
            "B-dummy_slot_name2",
            "O"
        ]);
    }
}
