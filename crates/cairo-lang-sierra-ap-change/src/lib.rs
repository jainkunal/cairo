//! Sierra AP change model.
use std::collections::HashMap;

use ap_change_info::ApChangeInfo;
use cairo_lang_sierra::extensions::core::{CoreLibfunc, CoreType};
use cairo_lang_sierra::extensions::ConcreteType;
use cairo_lang_sierra::ids::{ConcreteTypeId, FunctionId};
use cairo_lang_sierra::program::{Program, StatementIdx};
use cairo_lang_sierra::program_registry::{ProgramRegistry, ProgramRegistryError};
use core_libfunc_ap_change::ApChangeInfoProvider;
use generate_equations::{Effects, Var};
use thiserror::Error;

pub mod ap_change_info;
pub mod core_libfunc_ap_change;
mod generate_equations;

/// Describes the effect on the `ap` register in a given libfunc branch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApChange {
    /// The libfunc changes `ap` in an unknown way.
    Unknown,
    /// The libfunc changes `ap` by a known size.
    Known(usize),
    /// The libfunc changes `ap` by a known size, provided in the metadata. Currently this only
    /// includes `branch_align` libfunc.
    FromMetadata,
    /// The libfunc changes `ap` by a known size at locals finalization stage.
    AtLocalsFinalization(usize),
    /// The libfunc is a function call - it changes according to the given function and call cost.
    FunctionCall(FunctionId),
    /// The libfunc allocates locals, the `ap` change depends on the environment.
    FinalizeLocals,
}

/// Error occurring while calculating the costing of a program's variables.
#[derive(Error, Debug, Eq, PartialEq)]
pub enum ApChangeError {
    #[error("error from the program registry")]
    ProgramRegistryError(#[from] Box<ProgramRegistryError>),
    #[error("found an illegal statement index during ap change calculations")]
    StatementOutOfBounds(StatementIdx),
    #[error("found an illegal statement index during ap change calculations")]
    StatementOutOfOrder(StatementIdx),
    #[error("found an illegal invocation during cost calculations")]
    IllegalInvocation(StatementIdx),
    #[error("failed solving the ap changes")]
    SolvingApChangeEquationFailed,
}

impl ApChangeInfoProvider for ProgramRegistry<CoreType, CoreLibfunc> {
    fn type_size(&self, ty: &ConcreteTypeId) -> usize {
        self.get_type(ty).unwrap().info().size as usize
    }
}

/// Calculates gas information for a given program.
pub fn calc_ap_changes(program: &Program) -> Result<ApChangeInfo, ApChangeError> {
    let registry = ProgramRegistry::<CoreType, CoreLibfunc>::new(program)?;
    let equations = generate_equations::generate_equations(program, |libfunc_id| {
        let libfunc = registry.get_libfunc(libfunc_id)?;
        core_libfunc_ap_change::core_libfunc_ap_change(libfunc, &registry)
            .into_iter()
            .map(|ap_change| {
                Ok(match ap_change {
                    ApChange::AtLocalsFinalization(known) => {
                        Effects { ap_change: ApChange::Known(0), locals: known }
                    }
                    _ => Effects { ap_change, locals: 0 },
                })
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    let solution = cairo_lang_eq_solver::try_solve_equations(equations)
        .ok_or(ApChangeError::SolvingApChangeEquationFailed)?;

    let mut variable_values = HashMap::<StatementIdx, usize>::default();
    let mut function_ap_change = HashMap::<cairo_lang_sierra::ids::FunctionId, usize>::default();
    for (var, value) in solution {
        match var {
            Var::LibfuncImplicitApChangeVariable(idx) => {
                variable_values.insert(idx, value as usize)
            }
            Var::FunctionApChange(func_id) => function_ap_change.insert(func_id, value as usize),
        };
    }
    Ok(ApChangeInfo { variable_values, function_ap_change })
}