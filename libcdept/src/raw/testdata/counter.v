`timescale 1ns/1ps
module tb;
  reg clk = 0;
  reg [3:0] cnt = 0;
  always #5 clk = ~clk;
  always @(posedge clk) cnt <= cnt + 1;
  initial begin
    $dumpfile("c.fst");
    $dumpvars(0, tb);
    #100 $finish;
  end
endmodule
