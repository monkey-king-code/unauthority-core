// DEX Swap Screen — swap tokens inside a liquidity pool with quote preview.
//
// All amounts are integer strings — no floating-point.
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/api_service.dart';
import '../services/wallet_service.dart';
import '../models/dex_pool.dart';
import '../utils/log.dart';

class DexSwapScreen extends StatefulWidget {
  final DexPool pool;

  const DexSwapScreen({super.key, required this.pool});

  @override
  State<DexSwapScreen> createState() => _DexSwapScreenState();
}

class _DexSwapScreenState extends State<DexSwapScreen> {
  final _amountController = TextEditingController();
  bool _isTokenAInput = true; // true = sell A, buy B
  DexQuote? _quote;
  bool _isFetchingQuote = false;
  bool _isSwapping = false;

  @override
  void dispose() {
    _amountController.dispose();
    super.dispose();
  }

  String get _inputToken =>
      _isTokenAInput ? widget.pool.tokenA : widget.pool.tokenB;
  String get _outputToken =>
      _isTokenAInput ? widget.pool.tokenB : widget.pool.tokenA;

  Future<void> _fetchQuote() async {
    final amount = _amountController.text.trim();
    if (amount.isEmpty) {
      setState(() => _quote = null);
      return;
    }

    setState(() => _isFetchingQuote = true);
    try {
      final api = context.read<ApiService>();
      final quote = await api.getDexQuote(
        widget.pool.contractAddress,
        widget.pool.poolId,
        _inputToken,
        amount,
      );
      if (!mounted) return;
      setState(() {
        _quote = quote;
        _isFetchingQuote = false;
      });
    } catch (e) {
      losLog('❌ DEX quote error: $e');
      if (!mounted) return;
      setState(() {
        _quote = null;
        _isFetchingQuote = false;
      });
    }
  }

  Future<void> _executeSwap() async {
    final amount = _amountController.text.trim();
    if (amount.isEmpty || _quote == null) return;

    setState(() => _isSwapping = true);
    try {
      final api = context.read<ApiService>();
      final wallet = context.read<WalletService>();
      final walletInfo = await wallet.getCurrentWallet();
      final myAddr = walletInfo?['address'];
      if (myAddr == null) throw Exception('No wallet found');

      losLog(
          '🔄 [DEX] swap: $amount $_inputToken → $_outputToken in pool ${widget.pool.poolId}');

      // min_amount_out — apply 1% slippage tolerance.
      // Integer math: amountOut * 99 / 100
      final amountOutBig = BigInt.tryParse(_quote!.amountOut) ?? BigInt.zero;
      final minOut = (amountOutBig * BigInt.from(99)) ~/ BigInt.from(100);

      await api.callContract(
        contractAddress: widget.pool.contractAddress,
        function: 'swap',
        args: [
          widget.pool.poolId,
          _inputToken,
          amount,
          minOut.toString(),
        ],
        caller: myAddr,
      );

      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
              'Swapped $amount $_inputToken → ${_quote!.amountOut} $_outputToken'),
        ),
      );
      Navigator.pop(context);
    } catch (e) {
      losLog('❌ DEX swap error: $e');
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Swap failed: $e')),
      );
    } finally {
      if (mounted) setState(() => _isSwapping = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text('Swap — ${widget.pool.pairLabel}'),
      ),
      body: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Pool info card
            Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                  children: [
                    _ReserveColumn(
                        label: widget.pool.tokenA, value: widget.pool.reserveA),
                    const Icon(Icons.swap_horiz, size: 32, color: Colors.grey),
                    _ReserveColumn(
                        label: widget.pool.tokenB, value: widget.pool.reserveB),
                  ],
                ),
              ),
            ),
            const SizedBox(height: 16),

            // Direction toggles
            SegmentedButton<bool>(
              segments: [
                ButtonSegment(
                  value: true,
                  label: Text('Sell ${widget.pool.tokenA}'),
                ),
                ButtonSegment(
                  value: false,
                  label: Text('Sell ${widget.pool.tokenB}'),
                ),
              ],
              selected: {_isTokenAInput},
              onSelectionChanged: (v) {
                setState(() {
                  _isTokenAInput = v.first;
                  _quote = null;
                });
                _fetchQuote();
              },
            ),
            const SizedBox(height: 16),

            // Amount input
            TextField(
              controller: _amountController,
              keyboardType:
                  const TextInputType.numberWithOptions(decimal: false),
              decoration: InputDecoration(
                labelText: 'Amount In',
                suffixText: _inputToken,
                border: const OutlineInputBorder(),
              ),
              onChanged: (_) => _fetchQuote(),
            ),
            const SizedBox(height: 16),

            // Quote output
            Card(
              color: Colors.grey.shade900,
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: _isFetchingQuote
                    ? const Center(child: CircularProgressIndicator())
                    : _quote != null
                        ? Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Row(
                                mainAxisAlignment:
                                    MainAxisAlignment.spaceBetween,
                                children: [
                                  const Text('You Receive',
                                      style: TextStyle(color: Colors.grey)),
                                  Text('${_quote!.amountOut} $_outputToken',
                                      style: const TextStyle(
                                          fontSize: 20,
                                          fontWeight: FontWeight.bold)),
                                ],
                              ),
                              const SizedBox(height: 8),
                              Row(
                                mainAxisAlignment:
                                    MainAxisAlignment.spaceBetween,
                                children: [
                                  Text('Fee: ${_quote!.feeBps} bps',
                                      style: const TextStyle(
                                          fontSize: 12, color: Colors.grey)),
                                  Text(
                                      'Price Impact: ${_quote!.priceImpactDisplay}',
                                      style: TextStyle(
                                        fontSize: 12,
                                        color: _quote!.priceImpactBps > 300
                                            ? Colors.red
                                            : Colors.green,
                                      )),
                                ],
                              ),
                            ],
                          )
                        : const Center(
                            child: Text('Enter amount for quote',
                                style: TextStyle(color: Colors.grey))),
              ),
            ),
            const SizedBox(height: 24),

            // Swap button
            SizedBox(
              height: 52,
              child: ElevatedButton(
                onPressed: _isSwapping || _quote == null ? null : _executeSwap,
                child: _isSwapping
                    ? const SizedBox(
                        width: 24,
                        height: 24,
                        child: CircularProgressIndicator(strokeWidth: 2))
                    : const Text('Execute Swap',
                        style: TextStyle(fontSize: 16)),
              ),
            ),

            if (_quote != null && _quote!.priceImpactBps > 300) ...[
              const SizedBox(height: 8),
              const Text(
                '⚠ High price impact! Consider a smaller trade.',
                textAlign: TextAlign.center,
                style: TextStyle(color: Colors.orange, fontSize: 12),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _ReserveColumn extends StatelessWidget {
  final String label;
  final String value;

  const _ReserveColumn({required this.label, required this.value});

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        Text(label, style: const TextStyle(fontWeight: FontWeight.bold)),
        const SizedBox(height: 4),
        Text(value, style: const TextStyle(color: Colors.grey)),
      ],
    );
  }
}
