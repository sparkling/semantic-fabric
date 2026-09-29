//! Borrowed iterative walker for the structural-only generated-query screen.
//! Every stack push and every constant byte is charged before it is retained.

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, GraphPattern, OrderExpression,
    PropertyPathExpression,
};
use spargebra::term::{GroundTerm, Literal, NamedNodePattern, TermPattern, TriplePattern};

use super::{check_function, refuse, ShapeRefusal, ShapeRule};

enum Item<'a> {
    Pattern(&'a GraphPattern),
    Expression(&'a Expression),
    Path(&'a PropertyPathExpression),
    Order(&'a OrderExpression),
    Aggregate(&'a AggregateExpression),
    Triple(&'a TriplePattern),
    Term(&'a TermPattern),
    Predicate(&'a NamedNodePattern),
    Ground(&'a GroundTerm),
}

pub(super) struct Walk<'a, 'c> {
    control: &'c dyn QueryControl,
    stack: Vec<Item<'a>>,
    charged: u64,
}

impl<'a, 'c> Walk<'a, 'c> {
    pub(super) fn new(control: &'c dyn QueryControl) -> Self {
        Self {
            control,
            stack: Vec::new(),
            charged: 0,
        }
    }

    /// Walk to exhaustion; returns total charged compiler work.
    pub(super) fn run(mut self, root: &'a GraphPattern) -> Result<u64, ShapeRefusal> {
        self.push(Item::Pattern(root))?;
        while let Some(item) = self.stack.pop() {
            self.checkpoint()?;
            self.visit(item)?;
        }
        Ok(self.charged)
    }

    fn checkpoint(&self) -> Result<(), ShapeRefusal> {
        self.control.checkpoint().map_err(ShapeRefusal::Control)
    }

    fn terminate(&self, cause: QueryControlError) -> ShapeRefusal {
        ShapeRefusal::Control(self.control.terminate(cause))
    }

    fn overflow(&self) -> ShapeRefusal {
        self.terminate(QueryControlError::AccountingOverflow)
    }

    fn units(&self, len: usize) -> Result<u64, ShapeRefusal> {
        u64::try_from(len).map_err(|_| self.overflow())
    }

    fn charge(&mut self, units: u64) -> Result<(), ShapeRefusal> {
        self.control
            .consume(QueryCharge::CompilerWork, units)
            .map_err(ShapeRefusal::Control)?;
        let Some(total) = self.charged.checked_add(units) else {
            return Err(self.overflow());
        };
        self.charged = total;
        Ok(())
    }

    fn charge_bytes(&mut self, len: usize) -> Result<(), ShapeRefusal> {
        let units = self.units(len)?;
        self.charge(units)
    }

    fn charge_member(&mut self, len: usize) -> Result<(), ShapeRefusal> {
        let units = self.units(len)?;
        match units.checked_add(1) {
            Some(total) => self.charge(total),
            None => Err(self.overflow()),
        }
    }

    fn charge_literal(&mut self, literal: &Literal) -> Result<(), ShapeRefusal> {
        self.charge_bytes(literal.value().len())?;
        self.charge_bytes(literal.datatype().as_str().len())?;
        if let Some(language) = literal.language() {
            self.charge_bytes(language.len())?;
        }
        Ok(())
    }

    fn push(&mut self, item: Item<'a>) -> Result<(), ShapeRefusal> {
        self.charge(1)?;
        if self.stack.try_reserve(1).is_err() {
            return Err(self.terminate(QueryControlError::CompilerResourceExhausted));
        }
        self.stack.push(item);
        Ok(())
    }

    fn push_pattern(&mut self, pattern: &'a GraphPattern) -> Result<(), ShapeRefusal> {
        self.push(Item::Pattern(pattern))
    }

    fn push_expression(&mut self, expression: &'a Expression) -> Result<(), ShapeRefusal> {
        self.push(Item::Expression(expression))
    }

    fn push_expressions(&mut self, expressions: &'a [Expression]) -> Result<(), ShapeRefusal> {
        for expression in expressions {
            self.push_expression(expression)?;
        }
        Ok(())
    }

    fn push_binary(
        &mut self,
        left: &'a Expression,
        right: &'a Expression,
    ) -> Result<(), ShapeRefusal> {
        self.push_expression(left)?;
        self.push_expression(right)
    }

    fn push_pair(
        &mut self,
        left: &'a GraphPattern,
        right: &'a GraphPattern,
    ) -> Result<(), ShapeRefusal> {
        self.push_pattern(left)?;
        self.push_pattern(right)
    }

    fn push_path(&mut self, path: &'a PropertyPathExpression) -> Result<(), ShapeRefusal> {
        self.push(Item::Path(path))
    }

    fn visit(&mut self, item: Item<'a>) -> Result<(), ShapeRefusal> {
        match item {
            Item::Pattern(pattern) => self.visit_pattern(pattern),
            Item::Expression(expression) => self.visit_expression(expression),
            Item::Path(path) => self.visit_path(path),
            Item::Order(order) => self.visit_order(order),
            Item::Aggregate(aggregate) => self.visit_aggregate(aggregate),
            Item::Triple(triple) => self.visit_triple(triple),
            Item::Term(term) => self.visit_term(term),
            Item::Predicate(predicate) => self.visit_predicate(predicate),
            Item::Ground(term) => self.visit_ground(term),
        }
    }

    // The wildcard arm fails closed for feature-gated variants (e.g. Lateral)
    // that may or may not exist in the active spargebra build.
    #[allow(unreachable_patterns)]
    fn visit_pattern(&mut self, pattern: &'a GraphPattern) -> Result<(), ShapeRefusal> {
        match pattern {
            GraphPattern::Bgp { patterns } => {
                for triple in patterns {
                    self.push(Item::Triple(triple))?;
                }
                Ok(())
            }
            GraphPattern::Path {
                subject,
                path,
                object,
            } => {
                self.push(Item::Term(subject))?;
                self.push_path(path)?;
                self.push(Item::Term(object))
            }
            GraphPattern::Join { left, right } => self.push_pair(left, right),
            GraphPattern::Union { left, right } => self.push_pair(left, right),
            GraphPattern::Minus { left, right } => self.push_pair(left, right),
            GraphPattern::LeftJoin {
                left,
                right,
                expression,
            } => {
                self.push_pair(left, right)?;
                if let Some(expression) = expression {
                    self.push_expression(expression)?;
                }
                Ok(())
            }
            GraphPattern::Filter { expr, inner } => {
                self.push_expression(expr)?;
                self.push_pattern(inner)
            }
            GraphPattern::Graph { name, inner } => {
                self.push(Item::Predicate(name))?;
                self.push_pattern(inner)
            }
            GraphPattern::Extend {
                inner,
                variable,
                expression,
            } => {
                self.charge_member(variable.as_str().len())?;
                self.push_expression(expression)?;
                self.push_pattern(inner)
            }
            GraphPattern::Values {
                variables,
                bindings,
            } => {
                for variable in variables {
                    self.charge_member(variable.as_str().len())?;
                }
                for row in bindings {
                    self.charge(1)?;
                    for cell in row {
                        match cell {
                            Some(term) => self.push(Item::Ground(term))?,
                            None => self.charge(1)?,
                        }
                    }
                }
                Ok(())
            }
            GraphPattern::OrderBy { inner, expression } => {
                for order in expression {
                    self.push(Item::Order(order))?;
                }
                self.push_pattern(inner)
            }
            GraphPattern::Project { inner, variables } => {
                for variable in variables {
                    self.charge_member(variable.as_str().len())?;
                }
                self.push_pattern(inner)
            }
            GraphPattern::Distinct { inner } => self.push_pattern(inner),
            GraphPattern::Reduced { inner } => self.push_pattern(inner),
            GraphPattern::Slice { inner, .. } => self.push_pattern(inner),
            GraphPattern::Group {
                inner,
                variables,
                aggregates,
            } => {
                for variable in variables {
                    self.charge_member(variable.as_str().len())?;
                }
                for (variable, aggregate) in aggregates {
                    self.charge_member(variable.as_str().len())?;
                    self.push(Item::Aggregate(aggregate))?;
                }
                self.push_pattern(inner)
            }
            GraphPattern::Service { .. } => refuse(ShapeRule::ServiceInPattern),
            _ => refuse(ShapeRule::UnclassifiedForm),
        }
    }

    fn visit_expression(&mut self, expression: &'a Expression) -> Result<(), ShapeRefusal> {
        match expression {
            Expression::NamedNode(node) => self.charge_bytes(node.as_str().len()),
            Expression::Literal(literal) => self.charge_literal(literal),
            Expression::Variable(variable) => self.charge_bytes(variable.as_str().len()),
            Expression::Bound(variable) => self.charge_bytes(variable.as_str().len()),
            Expression::Or(left, right) => self.push_binary(left, right),
            Expression::And(left, right) => self.push_binary(left, right),
            Expression::Equal(left, right) => self.push_binary(left, right),
            Expression::SameTerm(left, right) => self.push_binary(left, right),
            Expression::Greater(left, right) => self.push_binary(left, right),
            Expression::GreaterOrEqual(left, right) => self.push_binary(left, right),
            Expression::Less(left, right) => self.push_binary(left, right),
            Expression::LessOrEqual(left, right) => self.push_binary(left, right),
            Expression::Add(left, right) => self.push_binary(left, right),
            Expression::Subtract(left, right) => self.push_binary(left, right),
            Expression::Multiply(left, right) => self.push_binary(left, right),
            Expression::Divide(left, right) => self.push_binary(left, right),
            Expression::In(left, list) => {
                self.push_expression(left)?;
                self.push_expressions(list)
            }
            Expression::UnaryPlus(inner) => self.push_expression(inner),
            Expression::UnaryMinus(inner) => self.push_expression(inner),
            Expression::Not(inner) => self.push_expression(inner),
            Expression::Exists(pattern) => self.push_pattern(pattern),
            Expression::If(first, second, third) => {
                self.push_expression(first)?;
                self.push_expression(second)?;
                self.push_expression(third)
            }
            Expression::Coalesce(list) => self.push_expressions(list),
            Expression::FunctionCall(function, arguments) => {
                check_function(function)?;
                self.push_expressions(arguments)
            }
        }
    }

    fn visit_path(&mut self, path: &'a PropertyPathExpression) -> Result<(), ShapeRefusal> {
        match path {
            PropertyPathExpression::NamedNode(node) => self.charge_bytes(node.as_str().len()),
            PropertyPathExpression::Reverse(inner) => self.push_path(inner),
            PropertyPathExpression::ZeroOrMore(inner) => self.push_path(inner),
            PropertyPathExpression::OneOrMore(inner) => self.push_path(inner),
            PropertyPathExpression::ZeroOrOne(inner) => self.push_path(inner),
            PropertyPathExpression::Sequence(left, right) => {
                self.push_path(left)?;
                self.push_path(right)
            }
            PropertyPathExpression::Alternative(left, right) => {
                self.push_path(left)?;
                self.push_path(right)
            }
            PropertyPathExpression::NegatedPropertySet(nodes) => {
                for node in nodes {
                    self.charge_member(node.as_str().len())?;
                }
                Ok(())
            }
        }
    }

    fn visit_order(&mut self, order: &'a OrderExpression) -> Result<(), ShapeRefusal> {
        match order {
            OrderExpression::Asc(expression) => self.push_expression(expression),
            OrderExpression::Desc(expression) => self.push_expression(expression),
        }
    }

    fn visit_aggregate(&mut self, aggregate: &'a AggregateExpression) -> Result<(), ShapeRefusal> {
        match aggregate {
            AggregateExpression::CountSolutions { .. } => Ok(()),
            AggregateExpression::FunctionCall { name, expr, .. } => {
                self.check_aggregate_function(name)?;
                self.push_expression(expr)
            }
        }
    }

    fn check_aggregate_function(&mut self, name: &AggregateFunction) -> Result<(), ShapeRefusal> {
        match name {
            AggregateFunction::Count => Ok(()),
            AggregateFunction::Sum => Ok(()),
            AggregateFunction::Avg => Ok(()),
            AggregateFunction::Min => Ok(()),
            AggregateFunction::Max => Ok(()),
            AggregateFunction::Sample => Ok(()),
            AggregateFunction::GroupConcat { separator } => match separator {
                Some(separator) => self.charge_bytes(separator.len()),
                None => Ok(()),
            },
            AggregateFunction::Custom(_) => refuse(ShapeRule::CustomAggregateUnsupported),
        }
    }

    fn visit_triple(&mut self, triple: &'a TriplePattern) -> Result<(), ShapeRefusal> {
        self.push(Item::Term(&triple.subject))?;
        self.push(Item::Predicate(&triple.predicate))?;
        self.push(Item::Term(&triple.object))
    }

    fn visit_term(&mut self, term: &'a TermPattern) -> Result<(), ShapeRefusal> {
        match term {
            TermPattern::NamedNode(node) => self.charge_bytes(node.as_str().len()),
            TermPattern::BlankNode(node) => self.charge_bytes(node.as_str().len()),
            TermPattern::Literal(literal) => self.charge_literal(literal),
            TermPattern::Variable(variable) => self.charge_bytes(variable.as_str().len()),
            TermPattern::Triple(_) => refuse(ShapeRule::RdfStarUnsupported),
        }
    }

    fn visit_predicate(&mut self, predicate: &'a NamedNodePattern) -> Result<(), ShapeRefusal> {
        match predicate {
            NamedNodePattern::NamedNode(node) => self.charge_bytes(node.as_str().len()),
            NamedNodePattern::Variable(variable) => self.charge_bytes(variable.as_str().len()),
        }
    }

    fn visit_ground(&mut self, term: &'a GroundTerm) -> Result<(), ShapeRefusal> {
        match term {
            GroundTerm::NamedNode(node) => self.charge_bytes(node.as_str().len()),
            GroundTerm::Literal(literal) => self.charge_literal(literal),
            GroundTerm::Triple(_) => refuse(ShapeRule::RdfStarUnsupported),
        }
    }
}
