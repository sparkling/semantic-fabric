use super::reader::{JsonBufferedSolutionsIterator, JsonInnerSolutionsParser};
use super::terms::JsonInnerTermReader;
use crate::error::QueryResultsSyntaxError;
use json_event_parser::JsonEvent;
use oxrdf::{Term, Variable};
use std::collections::HashMap;
use std::mem::take;

pub(super) enum JsonInnerQueryResults {
    Solutions {
        variables: Vec<Variable>,
        solutions: JsonInnerSolutions,
    },
    Boolean(bool),
}

pub(super) enum JsonInnerSolutions {
    Reader(JsonInnerSolutionsParser),
    Iterator(JsonBufferedSolutionsIterator),
}

pub(super) struct JsonInnerReader {
    state: JsonInnerReaderState,
    variables: Vec<Variable>,
    current_solution_variables: Vec<String>,
    current_solution_values: Vec<Term>,
    solutions: Vec<(Vec<String>, Vec<Term>)>,
    vars_read: bool,
    solutions_read: bool,
}

enum JsonInnerReaderState {
    Start,
    InRootObject,
    BeforeHead,
    InHead,
    BeforeVars,
    InVars,
    BeforeLinks,
    InLinks,
    BeforeResults,
    InResults,
    BeforeBindings,
    BeforeSolution,
    BetweenSolutionTerms,
    Term {
        reader: JsonInnerTermReader,
        variable: String,
    },
    AfterBindings,
    BeforeBoolean,
    Ignore {
        level: usize,
        after: JsonInnerReaderStateAfterIgnore,
    },
}

#[derive(Clone, Copy)]
enum JsonInnerReaderStateAfterIgnore {
    InRootObject,
    InHead,
    InResults,
    AfterBindings,
}

impl JsonInnerReader {
    pub(super) fn new() -> Self {
        Self {
            state: JsonInnerReaderState::Start,
            variables: Vec::new(),
            current_solution_variables: Vec::new(),
            current_solution_values: Vec::new(),
            solutions: Vec::new(),
            vars_read: false,
            solutions_read: false,
        }
    }

    pub(super) fn read_event(
        &mut self,
        event: JsonEvent<'_>,
    ) -> Result<Option<JsonInnerQueryResults>, QueryResultsSyntaxError> {
        match &mut self.state {
            JsonInnerReaderState::Start => {
                if event == JsonEvent::StartObject {
                    self.state = JsonInnerReaderState::InRootObject;
                    Ok(None)
                } else {
                    Err(QueryResultsSyntaxError::msg(
                        "SPARQL JSON results must be an object",
                    ))
                }
            }
            JsonInnerReaderState::InRootObject => match event {
                JsonEvent::ObjectKey(key) => match key.as_ref() {
                    "head" => {
                        self.state = JsonInnerReaderState::BeforeHead;
                        Ok(None)
                    }
                    "results" => {
                        self.state = JsonInnerReaderState::BeforeResults;
                        Ok(None)
                    }
                    "boolean" => {
                        self.state = JsonInnerReaderState::BeforeBoolean;
                        Ok(None)
                    }
                    _ => {
                        self.state = JsonInnerReaderState::Ignore {
                            level: 0,
                            after: JsonInnerReaderStateAfterIgnore::InRootObject,
                        };
                        Ok(None)
                    }
                },
                JsonEvent::EndObject => Err(QueryResultsSyntaxError::msg(
                    "SPARQL JSON results must contain a 'boolean' or a 'results' key",
                )),
                _ => unreachable!(),
            },
            JsonInnerReaderState::BeforeHead => {
                if event == JsonEvent::StartObject {
                    self.state = JsonInnerReaderState::InHead;
                    Ok(None)
                } else {
                    Err(QueryResultsSyntaxError::msg(
                        "SPARQL JSON results head must be an object",
                    ))
                }
            }
            JsonInnerReaderState::InHead => match event {
                JsonEvent::ObjectKey(key) => match key.as_ref() {
                    "vars" => {
                        self.state = JsonInnerReaderState::BeforeVars;
                        self.vars_read = true;
                        Ok(None)
                    }
                    "links" => {
                        self.state = JsonInnerReaderState::BeforeLinks;
                        Ok(None)
                    }
                    _ => {
                        self.state = JsonInnerReaderState::Ignore {
                            level: 0,
                            after: JsonInnerReaderStateAfterIgnore::InHead,
                        };
                        Ok(None)
                    }
                },
                JsonEvent::EndObject => {
                    self.state = JsonInnerReaderState::InRootObject;
                    Ok(None)
                }
                _ => unreachable!(),
            },
            JsonInnerReaderState::BeforeVars => {
                if event == JsonEvent::StartArray {
                    self.state = JsonInnerReaderState::InVars;
                    Ok(None)
                } else {
                    Err(QueryResultsSyntaxError::msg(
                        "SPARQL JSON results vars must be an array",
                    ))
                }
            }
            JsonInnerReaderState::InVars => match event {
                JsonEvent::String(variable) => match Variable::new(variable.clone()) {
                    Ok(var) => {
                        if self.variables.contains(&var) {
                            return Err(QueryResultsSyntaxError::msg(format!(
                                "The variable {var} is declared twice"
                            )));
                        }
                        self.variables.push(var);
                        Ok(None)
                    }
                    Err(e) => Err(QueryResultsSyntaxError::msg(format!(
                        "Invalid variable name '{variable}': {e}"
                    ))),
                },
                JsonEvent::EndArray => {
                    if self.solutions_read {
                        let mut mapping = HashMap::new();
                        for (i, var) in self.variables.iter().enumerate() {
                            mapping.insert(var.as_str().to_owned(), i);
                        }
                        Ok(Some(JsonInnerQueryResults::Solutions {
                            variables: take(&mut self.variables),
                            solutions: JsonInnerSolutions::Iterator(
                                JsonBufferedSolutionsIterator::new(
                                    mapping,
                                    take(&mut self.solutions).into_iter(),
                                ),
                            ),
                        }))
                    } else {
                        self.state = JsonInnerReaderState::InHead;
                        Ok(None)
                    }
                }
                _ => Err(QueryResultsSyntaxError::msg(
                    "Variables name in the vars array must be strings",
                )),
            },
            JsonInnerReaderState::BeforeLinks => {
                if event == JsonEvent::StartArray {
                    self.state = JsonInnerReaderState::InLinks;
                    Ok(None)
                } else {
                    Err(QueryResultsSyntaxError::msg(
                        "SPARQL JSON results links must be an array",
                    ))
                }
            }
            JsonInnerReaderState::InLinks => match event {
                JsonEvent::String(_) => Ok(None),
                JsonEvent::EndArray => {
                    self.state = JsonInnerReaderState::InHead;
                    Ok(None)
                }
                _ => Err(QueryResultsSyntaxError::msg(
                    "Links in the links array must be strings",
                )),
            },
            JsonInnerReaderState::BeforeResults => {
                if event == JsonEvent::StartObject {
                    self.state = JsonInnerReaderState::InResults;
                    Ok(None)
                } else {
                    Err(QueryResultsSyntaxError::msg(
                        "SPARQL JSON results result must be an object",
                    ))
                }
            }
            JsonInnerReaderState::InResults => match event {
                JsonEvent::ObjectKey(key) => {
                    if key == "bindings" {
                        self.state = JsonInnerReaderState::BeforeBindings;
                        Ok(None)
                    } else {
                        self.state = JsonInnerReaderState::Ignore {
                            level: 0,
                            after: JsonInnerReaderStateAfterIgnore::InResults,
                        };
                        Ok(None)
                    }
                }
                JsonEvent::EndObject => Err(QueryResultsSyntaxError::msg(
                    "The results object must contains a 'bindings' key",
                )),
                _ => unreachable!(),
            },
            JsonInnerReaderState::BeforeBindings => {
                if event == JsonEvent::StartArray {
                    self.solutions_read = true;
                    if self.vars_read {
                        let mut mapping = HashMap::new();
                        for (i, var) in self.variables.iter().enumerate() {
                            mapping.insert(var.as_str().to_owned(), i);
                        }
                        Ok(Some(JsonInnerQueryResults::Solutions {
                            variables: take(&mut self.variables),
                            solutions: JsonInnerSolutions::Reader(JsonInnerSolutionsParser::new(
                                mapping,
                            )),
                        }))
                    } else {
                        self.state = JsonInnerReaderState::BeforeSolution;
                        Ok(None)
                    }
                } else {
                    Err(QueryResultsSyntaxError::msg(
                        "SPARQL JSON results bindings must be an array",
                    ))
                }
            }
            JsonInnerReaderState::BeforeSolution => match event {
                JsonEvent::StartObject => {
                    self.state = JsonInnerReaderState::BetweenSolutionTerms;
                    Ok(None)
                }
                JsonEvent::EndArray => {
                    self.state = JsonInnerReaderState::AfterBindings;
                    Ok(None)
                }
                _ => Err(QueryResultsSyntaxError::msg(
                    "Expecting a new solution object",
                )),
            },
            JsonInnerReaderState::BetweenSolutionTerms => match event {
                JsonEvent::ObjectKey(key) => {
                    self.state = JsonInnerReaderState::Term {
                        reader: JsonInnerTermReader::default(),
                        variable: key.into(),
                    };
                    Ok(None)
                }
                JsonEvent::EndObject => {
                    self.state = JsonInnerReaderState::BeforeSolution;
                    self.solutions.push((
                        take(&mut self.current_solution_variables),
                        take(&mut self.current_solution_values),
                    ));
                    Ok(None)
                }
                _ => unreachable!(),
            },
            JsonInnerReaderState::Term { reader, variable } => {
                let result = reader.read_event(event);
                if let Some(term) = result? {
                    self.current_solution_variables.push(take(variable));
                    self.current_solution_values.push(term);
                    self.state = JsonInnerReaderState::BetweenSolutionTerms;
                }
                Ok(None)
            }
            JsonInnerReaderState::AfterBindings => {
                if event == JsonEvent::EndObject {
                    self.state = JsonInnerReaderState::InRootObject;
                } else {
                    self.state = JsonInnerReaderState::Ignore {
                        level: 0,
                        after: JsonInnerReaderStateAfterIgnore::AfterBindings,
                    }
                }
                Ok(None)
            }
            JsonInnerReaderState::BeforeBoolean => {
                if let JsonEvent::Boolean(v) = event {
                    Ok(Some(JsonInnerQueryResults::Boolean(v)))
                } else {
                    Err(QueryResultsSyntaxError::msg("Unexpected boolean value"))
                }
            }
            #[expect(clippy::ref_patterns)]
            &mut JsonInnerReaderState::Ignore {
                ref mut level,
                ref after,
            } => {
                let level = match event {
                    JsonEvent::StartArray | JsonEvent::StartObject => *level + 1,
                    JsonEvent::EndArray | JsonEvent::EndObject => *level - 1,
                    JsonEvent::String(_)
                    | JsonEvent::Number(_)
                    | JsonEvent::Boolean(_)
                    | JsonEvent::Null
                    | JsonEvent::ObjectKey(_)
                    | JsonEvent::Eof => *level,
                };
                self.state = if level == 0 {
                    match after {
                        JsonInnerReaderStateAfterIgnore::InRootObject => {
                            JsonInnerReaderState::InRootObject
                        }
                        JsonInnerReaderStateAfterIgnore::InHead => JsonInnerReaderState::InHead,
                        JsonInnerReaderStateAfterIgnore::InResults => {
                            JsonInnerReaderState::InResults
                        }
                        JsonInnerReaderStateAfterIgnore::AfterBindings => {
                            JsonInnerReaderState::AfterBindings
                        }
                    }
                } else {
                    JsonInnerReaderState::Ignore {
                        level,
                        after: *after,
                    }
                };
                Ok(None)
            }
        }
    }
}
