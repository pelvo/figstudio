# `.fig` Schema & Edit Coverage Census

- Files scanned: **10**
- Total nodes: **29574**
- Schema definitions: **666**
- `NodeChange` schema fields: **622** (observed active: **216**)

## Node Types

| Variant | Count |
|---|---:|
| `FRAME` | 9657 |
| `ELLIPSE` | 5162 |
| `VECTOR` | 4833 |
| `INSTANCE` | 3722 |
| `SYMBOL` | 1555 |
| `ROUNDED_RECTANGLE` | 1330 |
| `TEXT` | 1261 |
| `VARIABLE` | 880 |
| `BOOLEAN_OPERATION` | 773 |
| `KEYFRAME` | 116 |
| `VARIABLE_SET` | 72 |
| `KEYFRAME_TRACK` | 50 |
| `SECTION` | 42 |
| `BRUSH` | 25 |
| `CANVAS` | 20 |
| `CODE_COMPONENT` | 19 |
| `VARIABLE_OVERRIDE` | 18 |
| `ANIMATION_PRESET_INSTANCE` | 17 |
| `DOCUMENT` | 10 |
| `LINE` | 9 |
| `REGULAR_POLYGON` | 2 |
| `STAR` | 1 |

## Paint Types

| Variant | Count |
|---|---:|
| `SOLID` | 17816 |
| `IMAGE` | 362 |
| `PATTERN` | 18 |
| `GRADIENT_LINEAR` | 13 |
| `GRADIENT_ANGULAR` | 12 |
| `VIDEO` | 9 |
| `CUSTOM` | 4 |
| `GRADIENT_RADIAL` | 2 |

## Effect Types

| Variant | Count |
|---|---:|
| `DROP_SHADOW` | 186 |
| `INNER_SHADOW` | 43 |
| `CUSTOM` | 9 |
| `FOREGROUND_BLUR` | 2 |
| `GRAIN` | 1 |
| `NOISE` | 1 |

## Auto-Layout Stack Modes

| Variant | Count |
|---|---:|
| `HORIZONTAL` | 6875 |
| `VERTICAL` | 1026 |
| `GRID` | 51 |

## Boolean Operations

| Variant | Count |
|---|---:|
| `UNION` | 569 |
| `SUBTRACT` | 205 |
| `INTERSECT` | 5 |

## Style Types

| Variant | Count |
|---|---:|
| `TEXT` | 25 |
| `FILL` | 18 |
| `EFFECT` | 6 |
| `GRID` | 3 |

## Variable Resolved Types

| Variant | Count |
|---|---:|
| `COLOR` | 739 |
| `FLOAT` | 110 |
| `STRING` | 26 |
| `EASING` | 3 |
| `TIMING` | 2 |

## Observed `NodeChange` Fields

| Field | Kiwi Type | Observed Nodes |
|---|---|---:|
| `guid` | `GUID` | 29574 |
| `phase` | `NodePhase` | 29574 |
| `type` | `NodeType` | 29574 |
| `visible` | `bool` | 29574 |
| `parentIndex` | `ParentIndex` | 29564 |
| `name` | `string` | 29541 |
| `opacity` | `float` | 29204 |
| `strokeAlign` | `StrokeAlign` | 29159 |
| `strokeJoin` | `StrokeJoin` | 29159 |
| `strokeWeight` | `float` | 29159 |
| `transform` | `Matrix` | 29038 |
| `size` | `Vector` | 28408 |
| `frameMaskDisabled` | `bool` | 14976 |
| `fillPaints` | `Paint[]` | 14834 |
| `fillGeometry` | `Path[]` | 14549 |
| `overrideKey` | `GUID` | 13971 |
| `parameterConsumptionMap` | `VariableDataMap` | 8803 |
| `stackMode` | `StackMode` | 7952 |
| `stackCounterAlignItems` | `StackAlign` | 6560 |
| `stackCounterSizing` | `StackSize` | 6036 |
| `proportionsConstrained` | `bool` | 5914 |
| `arcData` | `ArcData` | 5260 |
| `horizontalConstraint` | `ConstraintType` | 5236 |
| `verticalConstraint` | `ConstraintType` | 5210 |
| `vectorData` | `VectorData` | 5000 |
| `componentPropRefs` | `ComponentPropRef[]` | 4573 |
| `userFacingVersion` | `string` | 4360 |
| `derivedSymbolData` | `NodeChange[]` | 3722 |
| `derivedSymbolDataLayoutVersion` | `int` | 3722 |
| `symbolData` | `SymbolData` | 3722 |
| `stackPrimarySizing` | `StackSize` | 3362 |
| `strokePaints` | `Paint[]` | 3325 |
| `styleIdForFill` | `StyleId` | 3311 |
| `stackSpacing` | `float` | 3196 |
| `stackPrimaryAlignItems` | `StackJustify` | 3139 |
| `strokeGeometry` | `Path[]` | 2942 |
| `rectangleTopRightCornerRadius` | `float` | 2797 |
| `rectangleBottomRightCornerRadius` | `float` | 2796 |
| `minSize` | `OptionalVector` | 2765 |
| `publishID` | `GUID` | 2658 |
| `rectangleBottomLeftCornerRadius` | `float` | 2628 |
| `rectangleTopLeftCornerRadius` | `float` | 2624 |
| `sourceLibraryKey` | `string` | 2623 |
| `rectangleCornerRadiiIndependent` | `bool` | 2503 |
| `pluginData` | `PluginData[]` | 2375 |
| `borderLeftWeight` | `float` | 2156 |
| `borderBottomWeight` | `float` | 1860 |
| `borderTopWeight` | `float` | 1858 |
| `borderRightWeight` | `float` | 1845 |
| `stackChildAlignSelf` | `StackCounterAlign` | 1782 |
| `componentKey` | `string` | 1639 |
| `isSymbolPublishable` | `bool` | 1555 |
| `sharedSymbolVersion` | `string` | 1555 |
| `stackPaddingRight` | `float` | 1509 |
| `stackChildPrimaryGrow` | `float` | 1501 |
| `symbolDescription` | `string` | 1498 |
| `locked` | `bool` | 1470 |
| `variantPropSpecs` | `VariantPropSpec[]` | 1447 |
| `description` | `string` | 1380 |
| `componentPropDefs` | `ComponentPropDef[]` | 1334 |
| `stackHorizontalPadding` | `float` | 1296 |
| `autoRename` | `bool` | 1263 |
| `fontName` | `FontName` | 1261 |
| `fontSize` | `float` | 1261 |
| `fontVariantCommonLigatures` | `bool` | 1261 |
| `fontVariantContextualLigatures` | `bool` | 1261 |
| `fontVersion` | `string` | 1261 |
| `letterSpacing` | `Number` | 1261 |
| `lineHeight` | `Number` | 1261 |
| `textAlignVertical` | `TextAlignVertical` | 1261 |
| `textBidiVersion` | `uint` | 1261 |
| `textData` | `TextData` | 1261 |
| `derivedTextData` | `DerivedTextData` | 1260 |
| `fontVariations` | `FontVariation[]` | 1253 |
| `textTracking` | `float` | 1248 |
| `textUserLayoutVersion` | `uint` | 1246 |
| `cornerRadius` | `float` | 1238 |
| `textExplicitLayoutVersion` | `uint` | 1215 |
| `textAutoResize` | `TextAutoResize` | 1191 |
| `version` | `string` | 1153 |
| `isPublishable` | `bool` | 1118 |
| `key` | `string` | 1066 |
| `libraryMoveHistory` | `LibraryMoveHistoryItem[]` | 1004 |
| `stackVerticalPadding` | `float` | 927 |
| `styleIdForText` | `StyleId` | 910 |
| `variableDataValues` | `VariableDataValues` | 898 |
| `variableSetID` | `VariableSetID` | 898 |
| `sortPosition` | `string` | 892 |
| `propsAreBubbled` | `bool` | 880 |
| `variableResolvedType` | `VariableResolvedDataType` | 880 |
| `strokeCap` | `StrokeCap` | 864 |
| `textDecorationSkipInk` | `bool` | 825 |
| `gridColumnAnchor` | `GUID` | 809 |
| `gridRowAnchor` | `GUID` | 809 |
| `booleanOperation` | `BooleanOperation` | 779 |
| `vectorOperationVersion` | `uint` | 770 |
| `codeSyntax` | `CodeSyntaxMap` | 727 |
| `stackPaddingBottom` | `float` | 698 |
| `stackPositioning` | `StackPositioning` | 635 |
| `detachedSymbolId` | `SymbolId` | 564 |
| `exportBackgroundDisabled` | `bool` | 520 |
| `prototypeInteractions` | `PrototypeInteraction[]` | 486 |
| `componentPropAssignments` | `ComponentPropAssignment[]` | 482 |
| `variableScopes` | `VariableScope[]` | 447 |
| `targetAspectRatio` | `OptionalVector` | 367 |
| `borderStrokeWeightsIndependent` | `bool` | 362 |
| `textAlignHorizontal` | `TextAlignHorizontal` | 335 |
| `isSlotContent` | `bool` | 333 |
| `resizeToFit` | `bool` | 306 |
| `paragraphSpacing` | `float` | 292 |
| `hyperlink` | `Hyperlink` | 280 |
| `emojiImageSet` | `EmojiImageSet` | 250 |
| `textTruncation` | `TextTruncation` | 235 |
| `bordersTakeSpace` | `bool` | 230 |
| `maxLines` | `int` | 188 |
| `dashPattern` | `float[]` | 167 |
| `lockMode` | `LockMode` | 161 |
| `effects` | `Effect[]` | 139 |
| `gridChildHorizontalAlign` | `GridChildAlign` | 130 |
| `easingData` | `EasingData` | 116 |
| `keyframeValue` | `KeyframeValueData` | 116 |
| `variableConsumptionMap` | `VariableDataMap` | 97 |
| `isStateGroup` | `bool` | 95 |
| `stateGroupPropertyValueOrders` | `StateGroupPropertyValueOrder[]` | 95 |
| `editInfo` | `EditInfo` | 87 |
| `blendMode` | `BlendMode` | 78 |
| `variableSetModes` | `VariableSetMode[]` | 72 |
| `timelinePosition` | `int64` | 69 |
| `isCollectionExtendable` | `bool` | 57 |
| `gridColumns` | `GUIDPositionMap` | 55 |
| `gridRows` | `GUIDPositionMap` | 55 |
| `gridColumnsSizing` | `GUIDGridTrackSizeMap` | 52 |
| `gridRowsSizing` | `GUIDGridTrackSizeMap` | 52 |
| `styleIdForEffect` | `StyleId` | 52 |
| `styleType` | `StyleType` | 52 |
| `stackCounterSpacing` | `float` | 48 |
| `gridColumnGap` | `float` | 47 |
| `gridRowGap` | `float` | 47 |
| `mask` | `bool` | 45 |
| `variableModeBySetMap` | `VariableModeBySetMap` | 41 |
| `cornerSmoothing` | `float` | 38 |
| `maskType` | `MaskType` | 37 |
| `stackLayoutVersion` | `uint` | 36 |
| `exportSettings` | `ExportSettings[]` | 35 |
| `miterLimit` | `float` | 30 |
| `pluginRelaunchData` | `PluginRelaunchData[]` | 28 |
| `stackWrap` | `StackWrap` | 28 |
| `gridAutoTracks` | `GridAutoTracks` | 27 |
| `gridReflowEnabled` | `bool` | 27 |
| `handleMirroring` | `VectorMirror` | 26 |
| `keyframeOperation` | `KeyframeOperation` | 26 |
| `brushType` | `BrushType` | 25 |
| `count` | `uint` | 23 |
| `annotations` | `Annotation[]` | 22 |
| `codeObjectType` | `CodeObjectType` | 19 |
| `overriddenVariableId` | `VariableID` | 18 |
| `backingCodeComponentId` | `CodeComponentId` | 17 |
| `codeSnapshot` | `CodeSnapshot` | 17 |
| `stackReverseZIndex` | `bool` | 17 |
| `measurements` | `AnnotationMeasurement[]` | 16 |
| `customToolArtifactRef` | `CustomToolArtifactRef` | 15 |
| `layoutGrids` | `LayoutGrid[]` | 14 |
| `styleDescription` | `string` | 14 |
| `styleIdForStrokeFill` | `StyleId` | 14 |
| `timelineOffset` | `int64` | 14 |
| `maxSize` | `OptionalVector` | 13 |
| `toggledOnOTFeatures` | `OpenTypeFeature[]` | 13 |
| `strokeSeed` | `uint64` | 12 |
| `backgroundColor` | `Color` | 10 |
| `backgroundEnabled` | `bool` | 10 |
| `backgroundOpacity` | `float` | 10 |
| `containerSupportsFillStrokeAndCorners` | `bool` | 10 |
| `gridRowSpan` | `uint` | 10 |
| `internalOnly` | `bool` | 10 |
| `isSoftDeleted` | `bool` | 10 |
| `scatterStrokeSettings` | `ScatterStrokeSettings` | 10 |
| `animationPresets` | `AnimationPresets` | 9 |
| `gridColumnSpan` | `uint` | 9 |
| `guides` | `Guide[]` | 9 |
| `maskIsOutline` | `bool` | 9 |
| `rectangleCornerToolIndependent` | `bool` | 8 |
| `styleIdForGrid` | `StyleId` | 8 |
| `documentColorProfile` | `DocumentColorProfile` | 7 |
| `motionPathBezierHandles` | `BezierHandles` | 7 |
| `symbolLinks` | `SymbolLink[]` | 7 |
| `gridChildVerticalAlign` | `GridChildAlign` | 6 |
| `timelineDuration` | `uint64` | 6 |
| `codeComponentDescription` | `string` | 4 |
| `codeExportName` | `string` | 4 |
| `hasBeenManuallyRenamed` | `bool` | 4 |
| `leadingTrim` | `LeadingTrim` | 4 |
| `rotationOrigin` | `Vector` | 4 |
| `simplifyInstancePanels` | `bool` | 4 |
| `backingVariableSetId` | `VariableSetID` | 3 |
| `inheritedVariableIds` | `InheritedVariablesData` | 3 |
| `timelineAssignments` | `TimelineAssignmentsMap` | 3 |
| `annotationCategories` | `AnnotationCategories` | 2 |
| `handoffStatusMap` | `HandoffStatusMap` | 2 |
| `starInnerScale` | `float` | 2 |
| `timelineDefinitions` | `TimelineDefinitionsMap` | 2 |
| `backgroundPaints` | `Paint[]` | 1 |
| `detachOpticalSizeFromFontSize` | `bool` | 1 |
| `fontVariantCaps` | `FontVariantCaps` | 1 |
| `fontVariantDiscretionaryLigatures` | `bool` | 1 |
| `fontVariantHistoricalLigatures` | `bool` | 1 |
| `fontVariantNumericFigure` | `FontVariantNumericFigure` | 1 |
| `fontVariantNumericFraction` | `FontVariantNumericFraction` | 1 |
| `fontVariantNumericSpacing` | `FontVariantNumericSpacing` | 1 |
| `fontVariantOrdinal` | `bool` | 1 |
| `fontVariantPosition` | `FontVariantPosition` | 1 |
| `fontVariantSlashedZero` | `bool` | 1 |
| `isPageDivider` | `bool` | 1 |
| `prototypeDevice` | `PrototypeDevice` | 1 |
| `scrollDirection` | `ScrollDirection` | 1 |
| `textWrapStyle` | `TextWrapStyle` | 1 |
| `toggledOffOTFeatures` | `OpenTypeFeature[]` | 1 |
