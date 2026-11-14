import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';

/// A small spatial entrance on section changes; the child keeps its state.
class SpringEntrance extends StatefulWidget {
  const SpringEntrance({super.key, required this.section, required this.child});

  final Object section;
  final Widget child;

  @override
  State<SpringEntrance> createState() => _SpringEntranceState();
}

class _SpringEntranceState extends State<SpringEntrance>
    with SingleTickerProviderStateMixin {
  late final _animation = AnimationController.unbounded(vsync: this);

  @override
  void didUpdateWidget(SpringEntrance oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.section == widget.section) return;
    if (MediaQuery.disableAnimationsOf(context)) {
      _animation.value = 0;
      return;
    }
    _animation.value = 1;
    _animation.animateWith(
      SpringSimulation(
        SpringDescription.withDampingRatio(mass: 1, stiffness: 380, ratio: .8),
        1,
        0,
        0,
      ),
    );
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _animation.stop();
      _animation.value = 0;
    }
  }

  @override
  void dispose() {
    _animation.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
    animation: _animation,
    child: widget.child,
    builder: (context, child) => Transform.translate(
      offset: Offset(0, 16 * _animation.value),
      child: child,
    ),
  );
}
