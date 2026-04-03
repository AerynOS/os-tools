// SPDX-FileCopyrightText: 2024 AerynOS Developers
// SPDX-License-Identifier: MPL-2.0

use stone_recipe::tuning::Toolchain;

use crate::architecture::BuildTarget;
use crate::recipe::Recipe;

pub fn stages(recipe: &Recipe, target: BuildTarget) -> Option<Vec<Stage>> {
    let build = recipe.build_target_definition(target);

    build.workload.is_some().then(|| {
        let mut stages = vec![Stage::One];

        if matches!(recipe.parsed.options.toolchain, Toolchain::Llvm) && recipe.parsed.options.cspgo {
            stages.push(Stage::Two);
        }

        stages.push(Stage::Use);

        stages
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
pub enum Stage {
    #[strum(serialize = "stage1")]
    One,
    #[strum(serialize = "stage1")]
    Two,
    #[strum(serialize = "use")]
    Use,
}
