use spargebra::algebra::{AggregateFunction, Expression, Function, GraphPattern};
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern};
use spargebra::Query;

use super::{CompileEnvelopeError, Frame, Validator, Work};

impl<'query> Validator<'query> {
    pub(super) fn visit(&mut self, frame: Frame<'query>) -> Result<(), CompileEnvelopeError> {
        let child_depth = frame.depth + 1;
        match frame.work {
            Work::Query(query) => self.query(query, child_depth),
            Work::Dataset(dataset) => {
                self.collection(dataset.default.len())?;
                for graph in dataset.default.iter().rev() {
                    self.push(child_depth, Work::NamedNode(graph))?;
                }
                if let Some(named) = &dataset.named {
                    self.collection(named.len())?;
                    for graph in named.iter().rev() {
                        self.push(child_depth, Work::NamedNode(graph))?;
                    }
                }
                Ok(())
            }
            Work::Graph(pattern) => self.graph(pattern, child_depth),
            Work::Expression(expression) => self.expression(expression, child_depth),
            Work::Path(path) => self.path(path, child_depth),
            Work::Aggregate(aggregate) => self.aggregate(aggregate, child_depth),
            Work::Order(order) => {
                let expression = match order {
                    spargebra::algebra::OrderExpression::Asc(expression)
                    | spargebra::algebra::OrderExpression::Desc(expression) => expression,
                };
                self.push(child_depth, Work::Expression(expression))
            }
            Work::Triple(triple) => {
                self.push(child_depth, Work::Term(&triple.object))?;
                self.push(child_depth, Work::NamedPattern(&triple.predicate))?;
                self.push(child_depth, Work::Term(&triple.subject))
            }
            Work::Term(term) => match term {
                TermPattern::NamedNode(node) => self.push(child_depth, Work::NamedNode(node)),
                TermPattern::BlankNode(node) => self.push(child_depth, Work::BlankNode(node)),
                TermPattern::Literal(literal) => self.push(child_depth, Work::Literal(literal)),
                TermPattern::Triple(triple) => self.push(child_depth, Work::Triple(triple)),
                TermPattern::Variable(variable) => self.push(child_depth, Work::Variable(variable)),
            },
            Work::NamedPattern(pattern) => match pattern {
                NamedNodePattern::NamedNode(node) => self.push(child_depth, Work::NamedNode(node)),
                NamedNodePattern::Variable(variable) => {
                    self.push(child_depth, Work::Variable(variable))
                }
            },
            Work::GroundTerm(term) => match term {
                GroundTerm::NamedNode(node) => self.push(child_depth, Work::NamedNode(node)),
                GroundTerm::Literal(literal) => self.push(child_depth, Work::Literal(literal)),
                GroundTerm::Triple(triple) => self.push(child_depth, Work::GroundTriple(triple)),
            },
            Work::GroundTriple(triple) => {
                self.push(child_depth, Work::GroundTerm(&triple.object))?;
                self.push(child_depth, Work::NamedNode(&triple.predicate))?;
                self.push(child_depth, Work::NamedNode(&triple.subject))
            }
            Work::NamedNode(node) => self.payload(node.as_str().len()),
            Work::Variable(variable) => self.payload(variable.as_str().len()),
            Work::BlankNode(node) => self.payload(node.as_str().len()),
            Work::Literal(literal) => {
                self.payload(literal.value().len())?;
                if let Some(language) = literal.language() {
                    self.payload(language.len())?;
                } else {
                    let datatype = literal.datatype();
                    if datatype.as_str() != "http://www.w3.org/2001/XMLSchema#string" {
                        self.payload(datatype.as_str().len())?;
                    }
                }
                Ok(())
            }
        }
    }

    fn query(&mut self, query: &'query Query, depth: usize) -> Result<(), CompileEnvelopeError> {
        let (dataset, pattern, base_iri) = match query {
            Query::Select {
                dataset,
                pattern,
                base_iri,
            }
            | Query::Describe {
                dataset,
                pattern,
                base_iri,
            }
            | Query::Ask {
                dataset,
                pattern,
                base_iri,
            } => (dataset, pattern, base_iri),
            Query::Construct {
                template,
                dataset,
                pattern,
                base_iri,
            } => {
                self.collection(template.len())?;
                for triple in template.iter().rev() {
                    self.push(depth, Work::Triple(triple))?;
                }
                (dataset, pattern, base_iri)
            }
        };
        if let Some(base_iri) = base_iri {
            self.payload(base_iri.as_str().len())?;
        }
        self.push(depth, Work::Graph(pattern))?;
        if let Some(dataset) = dataset {
            self.push(depth, Work::Dataset(dataset))?;
        }
        Ok(())
    }

    fn graph(
        &mut self,
        pattern: &'query GraphPattern,
        depth: usize,
    ) -> Result<(), CompileEnvelopeError> {
        match pattern {
            GraphPattern::Bgp { patterns } => {
                self.collection(patterns.len())?;
                for triple in patterns.iter().rev() {
                    self.push(depth, Work::Triple(triple))?;
                }
            }
            GraphPattern::Path {
                subject,
                path,
                object,
            } => {
                self.push(depth, Work::Term(object))?;
                self.push(depth, Work::Path(path))?;
                self.push(depth, Work::Term(subject))?;
            }
            GraphPattern::Join { left, right }
            | GraphPattern::Lateral { left, right }
            | GraphPattern::Union { left, right }
            | GraphPattern::Minus { left, right } => {
                self.push(depth, Work::Graph(right))?;
                self.push(depth, Work::Graph(left))?;
            }
            GraphPattern::LeftJoin {
                left,
                right,
                expression,
            } => {
                if let Some(expression) = expression {
                    self.push(depth, Work::Expression(expression))?;
                }
                self.push(depth, Work::Graph(right))?;
                self.push(depth, Work::Graph(left))?;
            }
            GraphPattern::Filter { expr, inner } => {
                self.push(depth, Work::Expression(expr))?;
                self.push(depth, Work::Graph(inner))?;
            }
            GraphPattern::Graph { name, inner } => {
                self.push(depth, Work::NamedPattern(name))?;
                self.push(depth, Work::Graph(inner))?;
            }
            GraphPattern::Extend {
                inner,
                variable,
                expression,
            } => {
                self.push(depth, Work::Expression(expression))?;
                self.push(depth, Work::Variable(variable))?;
                self.push(depth, Work::Graph(inner))?;
            }
            GraphPattern::Values {
                variables,
                bindings,
            } => {
                self.collection(variables.len())?;
                self.collection(bindings.len())?;
                for row in bindings.iter().rev() {
                    self.collection(row.len())?;
                    for value in row.iter().rev().flatten() {
                        self.push(depth, Work::GroundTerm(value))?;
                    }
                }
                for variable in variables.iter().rev() {
                    self.push(depth, Work::Variable(variable))?;
                }
            }
            GraphPattern::OrderBy { inner, expression } => {
                self.collection(expression.len())?;
                for order in expression.iter().rev() {
                    self.push(depth, Work::Order(order))?;
                }
                self.push(depth, Work::Graph(inner))?;
            }
            GraphPattern::Project { inner, variables } => {
                self.collection(variables.len())?;
                for variable in variables.iter().rev() {
                    self.push(depth, Work::Variable(variable))?;
                }
                self.push(depth, Work::Graph(inner))?;
            }
            GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. } => self.push(depth, Work::Graph(inner))?,
            GraphPattern::Group {
                inner,
                variables,
                aggregates,
            } => {
                self.collection(variables.len())?;
                self.collection(aggregates.len())?;
                for (variable, aggregate) in aggregates.iter().rev() {
                    self.push(depth, Work::Aggregate(aggregate))?;
                    self.push(depth, Work::Variable(variable))?;
                }
                for variable in variables.iter().rev() {
                    self.push(depth, Work::Variable(variable))?;
                }
                self.push(depth, Work::Graph(inner))?;
            }
            GraphPattern::Service { name, inner, .. } => {
                self.push(depth, Work::NamedPattern(name))?;
                self.push(depth, Work::Graph(inner))?;
            }
        }
        Ok(())
    }

    fn expression(
        &mut self,
        expression: &'query Expression,
        depth: usize,
    ) -> Result<(), CompileEnvelopeError> {
        match expression {
            Expression::NamedNode(node) => self.push(depth, Work::NamedNode(node))?,
            Expression::Literal(literal) => self.push(depth, Work::Literal(literal))?,
            Expression::Variable(variable) | Expression::Bound(variable) => {
                self.push(depth, Work::Variable(variable))?;
            }
            Expression::Or(left, right)
            | Expression::And(left, right)
            | Expression::Equal(left, right)
            | Expression::SameTerm(left, right)
            | Expression::Greater(left, right)
            | Expression::GreaterOrEqual(left, right)
            | Expression::Less(left, right)
            | Expression::LessOrEqual(left, right)
            | Expression::Add(left, right)
            | Expression::Subtract(left, right)
            | Expression::Multiply(left, right)
            | Expression::Divide(left, right) => {
                self.push(depth, Work::Expression(right))?;
                self.push(depth, Work::Expression(left))?;
            }
            Expression::In(left, right) => {
                self.collection(right.len())?;
                for expression in right.iter().rev() {
                    self.push(depth, Work::Expression(expression))?;
                }
                self.push(depth, Work::Expression(left))?;
            }
            Expression::UnaryPlus(inner)
            | Expression::UnaryMinus(inner)
            | Expression::Not(inner) => self.push(depth, Work::Expression(inner))?,
            Expression::Exists(pattern) => self.push(depth, Work::Graph(pattern))?,
            Expression::If(first, second, third) => {
                self.push(depth, Work::Expression(third))?;
                self.push(depth, Work::Expression(second))?;
                self.push(depth, Work::Expression(first))?;
            }
            Expression::Coalesce(expressions) => {
                self.collection(expressions.len())?;
                for expression in expressions.iter().rev() {
                    self.push(depth, Work::Expression(expression))?;
                }
            }
            Expression::FunctionCall(function, arguments) => {
                self.collection(arguments.len())?;
                self.function(function, depth)?;
                for argument in arguments.iter().rev() {
                    self.push(depth, Work::Expression(argument))?;
                }
            }
        }
        Ok(())
    }

    fn path(
        &mut self,
        path: &'query spargebra::algebra::PropertyPathExpression,
        depth: usize,
    ) -> Result<(), CompileEnvelopeError> {
        use spargebra::algebra::PropertyPathExpression as Path;
        match path {
            Path::NamedNode(node) => self.push(depth, Work::NamedNode(node))?,
            Path::Reverse(inner)
            | Path::ZeroOrMore(inner)
            | Path::OneOrMore(inner)
            | Path::ZeroOrOne(inner) => self.push(depth, Work::Path(inner))?,
            Path::Sequence(left, right) | Path::Alternative(left, right) => {
                self.push(depth, Work::Path(right))?;
                self.push(depth, Work::Path(left))?;
            }
            Path::NegatedPropertySet(nodes) => {
                self.collection(nodes.len())?;
                for node in nodes.iter().rev() {
                    self.push(depth, Work::NamedNode(node))?;
                }
            }
        }
        Ok(())
    }

    fn aggregate(
        &mut self,
        aggregate: &'query spargebra::algebra::AggregateExpression,
        depth: usize,
    ) -> Result<(), CompileEnvelopeError> {
        match aggregate {
            spargebra::algebra::AggregateExpression::CountSolutions { .. } => Ok(()),
            spargebra::algebra::AggregateExpression::FunctionCall { name, expr, .. } => {
                self.aggregate_function(name, depth)?;
                self.push(depth, Work::Expression(expr))
            }
        }
    }

    fn function(
        &mut self,
        function: &'query Function,
        depth: usize,
    ) -> Result<(), CompileEnvelopeError> {
        match function {
            Function::Str
            | Function::Lang
            | Function::LangMatches
            | Function::Datatype
            | Function::Iri
            | Function::BNode
            | Function::Rand
            | Function::Abs
            | Function::Ceil
            | Function::Floor
            | Function::Round
            | Function::Concat
            | Function::SubStr
            | Function::StrLen
            | Function::Replace
            | Function::UCase
            | Function::LCase
            | Function::EncodeForUri
            | Function::Contains
            | Function::StrStarts
            | Function::StrEnds
            | Function::StrBefore
            | Function::StrAfter
            | Function::Year
            | Function::Month
            | Function::Day
            | Function::Hours
            | Function::Minutes
            | Function::Seconds
            | Function::Timezone
            | Function::Tz
            | Function::Now
            | Function::Uuid
            | Function::StrUuid
            | Function::Md5
            | Function::Sha1
            | Function::Sha256
            | Function::Sha384
            | Function::Sha512
            | Function::StrLang
            | Function::StrDt
            | Function::IsIri
            | Function::IsBlank
            | Function::IsLiteral
            | Function::IsNumeric
            | Function::Regex
            | Function::Triple
            | Function::Subject
            | Function::Predicate
            | Function::Object
            | Function::IsTriple
            | Function::LangDir
            | Function::HasLang
            | Function::HasLangDir
            | Function::StrLangDir
            | Function::Adjust => Ok(()),
            Function::Custom(node) => self.push(depth, Work::NamedNode(node)),
        }
    }

    fn aggregate_function(
        &mut self,
        function: &'query AggregateFunction,
        depth: usize,
    ) -> Result<(), CompileEnvelopeError> {
        match function {
            AggregateFunction::Count
            | AggregateFunction::Sum
            | AggregateFunction::Avg
            | AggregateFunction::Min
            | AggregateFunction::Max
            | AggregateFunction::Sample => Ok(()),
            AggregateFunction::GroupConcat { separator } => {
                if let Some(separator) = separator {
                    self.payload(separator.len())?;
                }
                Ok(())
            }
            AggregateFunction::Custom(node) => self.push(depth, Work::NamedNode(node)),
        }
    }
}
